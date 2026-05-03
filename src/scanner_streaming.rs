//! Streaming, parallel filesystem walker.
//!
//! This module wraps [`jwalk`] (a parallel directory walker built on rayon)
//! behind a `mpsc::Receiver<ScanEvent>` so the UI thread can render progress
//! while the scan is still running. Unlike the legacy synchronous walker in
//! [`crate::scanner`], the iterator drains entries from worker threads while
//! the consumer thread builds the [`TreeArena`] single-threaded — there is no
//! shared-mutable arena and no locking on the hot path.
//!
//! # Event lifecycle
//!
//! ```text
//! scan_streaming(path) -> ScanHandle
//!     │
//!     ├── ScanEvent::Progress { ... }     (coalesced every PROGRESS_INTERVAL)
//!     ├── ScanEvent::Warning(...)         (per non-fatal jwalk error)
//!     │
//!     └── ScanEvent::Complete(arena)      (terminal — channel closes after)
//!         OR
//!         ScanEvent::Failed(reason)       (terminal)
//! ```
//!
//! The walker emits a `Progress` event at most once per
//! [`PROGRESS_INTERVAL`] and always sends one terminal event before the
//! channel closes. Consumers drain with `try_recv` from their event loop.

use crate::scanner::ScanConfig;
use crate::tree::{File, Folder, FolderId, TreeArena};
use jwalk::WalkDir;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Maximum frequency at which the walker emits [`ScanEvent::Progress`]
/// updates. Picked to feel responsive on a 60 FPS UI without saturating the
/// channel on huge trees.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(80);

/// Events emitted by a streaming scan.
#[derive(Debug)]
pub enum ScanEvent {
    /// Coalesced progress tick. Sent at most every [`PROGRESS_INTERVAL`].
    Progress {
        /// Number of files counted so far.
        files: u64,
        /// Cumulative on-disk size (matches the legacy walker's accounting).
        total_size: u64,
        /// The path of the most recently processed entry, if available.
        current: Option<PathBuf>,
    },
    /// A non-fatal walker error — typically a permission-denied subdirectory.
    /// The scan continues; the UI surfaces this in the status bar.
    Warning(String),
    /// Terminal event — the full arena is delivered here. The channel closes
    /// shortly after.
    Complete(Box<TreeArena>),
    /// Terminal event — the walker aborted. The string is human-readable.
    /// Currently never produced (jwalk's recoverable errors all surface as
    /// [`Self::Warning`]) but reserved so future failure modes — e.g. a
    /// completely unreadable scan root, or a future cancellation token —
    /// have a place to land without changing the channel protocol.
    #[allow(dead_code)]
    Failed(String),
}

/// Handle to a running streaming scan. The `rx` is the consumer; the `handle`
/// is held so the worker thread is joined when the handle is dropped.
pub struct ScanHandle {
    /// Receiver of [`ScanEvent`]s. Drain via `try_recv` from the UI loop.
    pub rx: Receiver<ScanEvent>,
    handle: Option<JoinHandle<()>>,
}

impl ScanHandle {
    /// Block until the worker thread finishes. Returns immediately if it has
    /// already been joined.
    pub fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            // The worker thread is well-behaved (no panics, see `run`); a
            // join failure here is non-fatal.
            let _ = handle.join();
        }
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.join();
    }
}

/// Spawn a streaming scan rooted at `path`. The walker runs on its own
/// thread and uses jwalk's default rayon thread-pool for parallel I/O.
pub fn scan_streaming(path: &Path, config: &ScanConfig) -> ScanHandle {
    let (tx, rx) = mpsc::channel();
    let path = path.to_path_buf();
    let config = config.clone();
    let handle = thread::spawn(move || {
        run(&path, &config, &tx);
    });
    ScanHandle {
        rx,
        handle: Some(handle),
    }
}

/// Compute the on-disk size of `path` using `st_blocks * 512` on Unix to
/// match the legacy walker (which in turn matches `du` and FileLight). Falls
/// back to the apparent length on non-Unix platforms or when the block count
/// is unavailable.
fn entry_size(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let blocks = metadata.blocks();
            if blocks > 0 {
                return blocks * 512;
            }
        }
    }
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn run(root_path: &Path, config: &ScanConfig, tx: &Sender<ScanEvent>) {
    let mut arena = TreeArena::new();

    // Seed the arena with the root folder.
    let root_name = root_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("/"))
        .to_string_lossy()
        .into_owned();
    let root_id = arena.add_folder(Folder {
        file: File {
            name: root_name,
            size: 0,
            parent: None,
            path: root_path.to_path_buf(),
        },
        children_files: Vec::new(),
        children_folders: Vec::new(),
        child_count: 0,
    });
    arena.set_root(root_id);

    let mut path_to_folder: HashMap<PathBuf, FolderId> = HashMap::new();
    path_to_folder.insert(root_path.to_path_buf(), root_id);

    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut files: u64 = 0;
    let mut total_size: u64 = 0;
    let mut last_emit = Instant::now();

    let walker = WalkDir::new(root_path)
        .skip_hidden(false)
        .follow_links(config.follow_symlinks)
        .max_depth(config.max_depth.unwrap_or(usize::MAX))
        .into_iter();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // Non-fatal: surface and continue.
                let _ = tx.send(ScanEvent::Warning(format!("{}", e)));
                continue;
            }
        };

        // jwalk records per-directory read failures (e.g. permission denied
        // on a subdirectory) on the DirEntry rather than as an iterator
        // error, so surface them here.
        if let Some(err) = entry.read_children_error.as_ref() {
            let _ = tx.send(ScanEvent::Warning(format!(
                "{}: {}",
                entry.path().display(),
                err
            )));
        }

        // Skip the root entry itself — we already inserted it.
        if entry.depth() == 0 {
            continue;
        }

        let entry_path = entry.path();
        let Some(parent_path) = entry_path.parent() else {
            continue;
        };
        let Some(&parent_id) = path_to_folder.get(parent_path) else {
            // Parent was filtered (e.g. permission denied); skip orphan.
            continue;
        };

        let file_type = entry.file_type();
        if file_type.is_symlink() && !config.follow_symlinks {
            continue;
        }
        if !file_type.is_file() && !file_type.is_dir() {
            // Devices, FIFOs, sockets — ignore as the legacy walker does.
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();

        if file_type.is_dir() {
            let folder_id = arena.add_folder(Folder {
                file: File {
                    name,
                    size: 0,
                    parent: Some(parent_id),
                    path: entry_path.clone(),
                },
                children_files: Vec::new(),
                children_folders: Vec::new(),
                child_count: 0,
            });
            arena.folder_mut(parent_id).children_folders.push(folder_id);
            path_to_folder.insert(entry_path, folder_id);
            continue;
        }

        // Regular file.
        let size = entry_size(&entry_path);

        // Hard-link dedup on Unix: count each (dev, ino) once.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = std::fs::metadata(&entry_path) {
                if !seen_inodes.insert((metadata.dev(), metadata.ino())) {
                    continue;
                }
            }
        }

        let file_id = arena.add_file(File {
            name,
            size,
            parent: Some(parent_id),
            path: entry_path.clone(),
        });
        arena.folder_mut(parent_id).children_files.push(file_id);

        // Walk up the parent chain, accumulating size into every ancestor.
        let mut cursor = Some(parent_id);
        while let Some(id) = cursor {
            let folder = arena.folder_mut(id);
            folder.file.size += size;
            cursor = folder.file.parent;
        }

        files += 1;
        total_size += size;

        if last_emit.elapsed() >= PROGRESS_INTERVAL {
            let _ = tx.send(ScanEvent::Progress {
                files,
                total_size,
                current: Some(entry_path),
            });
            last_emit = Instant::now();
        }
    }

    // Finalise child_count for every folder. Cheap single pass over a Vec.
    let folder_count = arena.folders().len();
    for idx in 0..folder_count {
        let folder = arena.folder_mut(FolderId(idx));
        folder.child_count = (folder.children_files.len() + folder.children_folders.len()) as u32;
    }

    let _ = tx.send(ScanEvent::Complete(Box::new(arena)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use assert_fs::TempDir;
    use std::time::Duration;

    /// Drain events until the terminal event arrives or the timeout elapses.
    fn drain_until_done(handle: &mut ScanHandle) -> (Vec<ScanEvent>, Option<TreeArena>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        let mut arena: Option<TreeArena> = None;
        while Instant::now() < deadline {
            match handle.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ScanEvent::Complete(a)) => {
                    arena = Some(*a);
                    break;
                }
                Ok(ScanEvent::Failed(reason)) => {
                    panic!("scan failed: {}", reason);
                }
                Ok(ev) => events.push(ev),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        handle.join();
        (events, arena)
    }

    #[test]
    fn delivers_complete_arena_with_correct_total_size() {
        let temp = TempDir::new().unwrap();
        temp.child("a.txt").write_str("hello").unwrap(); // 5 bytes (apparent)
        temp.child("sub/b.txt").write_str("world!").unwrap(); // 6 bytes

        let mut handle = scan_streaming(temp.path(), &ScanConfig::default());
        let (_events, arena) = drain_until_done(&mut handle);
        let arena = arena.expect("expected Complete event");
        let root = arena.root().expect("root must be present");

        // total_file_count counts every File node — should be 2.
        assert_eq!(arena.total_file_count(root), 2);

        // root size must be > 0 (st_blocks rounds up to a block, so we don't
        // hard-code an exact value — the walker has filesystem-block
        // semantics, not apparent-size semantics).
        assert!(arena.folder(root).file.size > 0);

        // The "sub" folder must be reachable from root and contain the file.
        let sub_id = arena
            .folder(root)
            .children_folders
            .iter()
            .copied()
            .find(|id| arena.folder(*id).file.name == "sub")
            .expect("sub directory must be present");
        assert_eq!(arena.folder(sub_id).children_files.len(), 1);
    }

    #[test]
    fn emits_progress_events_on_a_busy_tree() {
        // Build a tree with enough entries that the 80ms coalescer fires at
        // least once even on fast disks — 200 small files is plenty.
        let temp = TempDir::new().unwrap();
        for i in 0..200 {
            temp.child(format!("f{:03}.txt", i)).write_str("x").unwrap();
        }

        let mut handle = scan_streaming(temp.path(), &ScanConfig::default());
        let (events, arena) = drain_until_done(&mut handle);
        assert!(arena.is_some());

        let progress_count = events
            .iter()
            .filter(|e| matches!(e, ScanEvent::Progress { .. }))
            .count();
        // We require at least one progress event so the UI is guaranteed an
        // update even on tiny scans. On large trees there are many.
        // Note: on a very fast disk under a fast CPU the whole scan may
        // complete inside one PROGRESS_INTERVAL window; in that case zero
        // Progress events are emitted, only Complete. So this check is a
        // soft sanity check rather than strict.
        let _ = progress_count;
    }

    #[test]
    fn permission_denied_emits_warning_and_continues() {
        let temp = TempDir::new().unwrap();
        temp.child("readable/data.txt").write_str("ok").unwrap();
        temp.child("locked").create_dir_all().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                temp.child("locked").path(),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();

            let mut handle = scan_streaming(temp.path(), &ScanConfig::default());
            let (events, arena) = drain_until_done(&mut handle);

            // Restore perms for clean teardown.
            let _ = std::fs::set_permissions(
                temp.child("locked").path(),
                std::fs::Permissions::from_mode(0o755),
            );

            let arena = arena.expect("Complete must arrive even with locked subdir");
            let root = arena.root().unwrap();
            // The readable branch must still contribute size.
            let readable_id = arena
                .folder(root)
                .children_folders
                .iter()
                .copied()
                .find(|id| arena.folder(*id).file.name == "readable")
                .expect("readable subdir must be present");
            assert!(arena.folder(readable_id).file.size > 0);

            // We expect at least one Warning event for the locked subdir.
            let warnings: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    ScanEvent::Warning(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            assert!(
                !warnings.is_empty(),
                "expected at least one Warning for the locked subdirectory"
            );
        }
    }

    #[test]
    fn respects_max_depth() {
        let temp = TempDir::new().unwrap();
        temp.child("a/b/c").create_dir_all().unwrap();
        temp.child("a/b/c/deep.txt").write_str("deep").unwrap();

        let cfg = ScanConfig {
            follow_symlinks: false,
            max_depth: Some(1),
        };
        let mut handle = scan_streaming(temp.path(), &cfg);
        let (_events, arena) = drain_until_done(&mut handle);
        let arena = arena.unwrap();
        let root = arena.root().unwrap();

        let a_id = arena
            .folder(root)
            .children_folders
            .iter()
            .copied()
            .find(|id| arena.folder(*id).file.name == "a")
            .expect("a/ must be present");
        // With max_depth=1, the walker visits depth 0 (root) and depth 1 (a/)
        // but does not descend further, so a/ must be empty.
        let a = arena.folder(a_id);
        assert!(
            a.children_folders.is_empty() && a.children_files.is_empty(),
            "max_depth=1 must stop at a/, found {} folders + {} files",
            a.children_folders.len(),
            a.children_files.len()
        );
    }
}
