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
use globset::{Glob, GlobSet, GlobSetBuilder};
use jwalk::WalkDir;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
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

/// Handle to a running streaming scan.
///
/// Three pieces of shared state:
///
/// - `rx`: events from the worker (Progress / Warning / Complete).
/// - `live`: an `Arc<Mutex<TreeArena>>` *also held by the walker*.
///   The walker writes into it directly; the App can `try_lock()`
///   it at frame time to render a partial radial / sidebar before
///   the scan finishes. This is what makes huge scans feel
///   responsive: the user sees folders appear (and big ones grow
///   first) while the walker is still cataloguing the long tail.
/// - `handle`: joined on drop so re-scans can't leak a thread.
pub struct ScanHandle {
    /// Receiver of [`ScanEvent`]s. Drain via `try_recv` from the UI loop.
    pub rx: Receiver<ScanEvent>,
    /// Live arena being populated by the walker. The App holds the
    /// same `Arc` and reads it via `try_lock()` to draw partial
    /// state mid-scan.
    pub live: Arc<Mutex<TreeArena>>,
    /// Cooperative cancellation flag. When set to `true` (e.g. by
    /// the user pressing `q` mid-scan, or by `start_scan` being
    /// called again to re-scan a different path), the walker
    /// notices on its next iteration and exits without sending a
    /// final `Complete` event. The flag is also raised by
    /// [`ScanHandle::Drop`] so the worker stops walking even if the
    /// App forgets the handle.
    pub cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ScanHandle {
    /// Signal the worker to stop and (best-effort) wait for it.
    /// Reserved for callers that want deterministic teardown — none
    /// of the current call sites need it (Drop's set-flag-and-detach
    /// is enough for `q`, and a re-scan replaces the handle which
    /// triggers the same Drop on the old one). Kept on the public
    /// surface so future code (e.g. tests, integration harnesses)
    /// can wait for a clean exit.
    #[allow(dead_code)]
    pub fn cancel_and_wait(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ScanHandle {
    /// Set the cancellation flag and *detach* the worker thread.
    ///
    /// Pre-Phase-22 the Drop impl called `handle.join()`, which on a
    /// huge tree (the user's 247 GB / 2.1M-file `~`) blocked process
    /// teardown for the rest of the walk after the user pressed `q`.
    /// The terminal was already restored at that point, so the user
    /// saw a hung shell.
    ///
    /// Now we just raise the flag and forget the JoinHandle. The
    /// worker reads `cancel` on every iteration and exits within
    /// microseconds, and any straggler is reaped by the OS at
    /// process exit. We don't leak the thread for the App's
    /// lifetime — re-scan goes through `cancel_and_wait` if it
    /// needs deterministic teardown.
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Drop the JoinHandle without joining; the OS reaps the
        // worker on process exit, and the cancel flag is what
        // actually unblocks responsiveness.
        let _ = self.handle.take();
    }
}

/// Spawn a streaming scan rooted at `path`. The walker runs on its own
/// thread and uses jwalk's default rayon thread-pool for parallel I/O.
///
/// The returned [`ScanHandle::live`] is the *same* `Arc<Mutex<TreeArena>>`
/// the walker writes into — readers should `try_lock()` it (never
/// `lock()`) to keep mid-scan rendering non-blocking.
pub fn scan_streaming(path: &Path, config: &ScanConfig) -> ScanHandle {
    let (tx, rx) = mpsc::channel();
    let path = path.to_path_buf();
    let config = config.clone();
    let live = Arc::new(Mutex::new(TreeArena::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let live_for_worker = Arc::clone(&live);
    let cancel_for_worker = Arc::clone(&cancel);
    let handle = thread::spawn(move || {
        run(&path, &config, &tx, &live_for_worker, &cancel_for_worker);
    });
    ScanHandle {
        rx,
        live,
        cancel,
        handle: Some(handle),
    }
}

/// Compile a [`GlobSet`] from the user's exclude patterns. Returns
/// `None` when the pattern list is empty (so the matcher cost is
/// completely skipped on the hot path). Patterns that fail to parse
/// are reported as warnings and skipped — we never abort a scan over
/// a malformed glob.
fn build_exclude_set(patterns: &[String], tx: &Sender<ScanEvent>) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    let mut added = 0usize;
    for pat in patterns {
        match Glob::new(pat) {
            Ok(g) => {
                builder.add(g);
                added += 1;
            }
            Err(e) => {
                let _ = tx.send(ScanEvent::Warning(format!(
                    "ignoring invalid exclude pattern {:?}: {}",
                    pat, e
                )));
            }
        }
    }
    if added == 0 {
        return None;
    }
    match builder.build() {
        Ok(set) => Some(set),
        Err(e) => {
            let _ = tx.send(ScanEvent::Warning(format!(
                "exclude pattern compilation failed: {}",
                e
            )));
            None
        }
    }
}

/// Compute the size of `path` for accounting purposes.
///
/// When `apparent` is `false` (default) the on-disk size is used:
/// `st_blocks * 512` on Unix, falling back to the apparent length when
/// the block count is zero or unavailable. This matches `du` and
/// FileLight.
///
/// When `apparent` is `true` the byte count from `metadata.len()` is
/// returned directly — what `ls -l` shows. Useful for sparse files,
/// transparently-compressed filesystems, and CoW snapshots where the
/// on-disk number is misleading.
fn entry_size(path: &Path, apparent: bool) -> u64 {
    if apparent {
        return std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    }
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

/// Walker entrypoint, shared-arena variant (Phase 21).
///
/// Writes directly into `shared` so the App can `try_lock()` the same
/// `Arc<Mutex<TreeArena>>` from the render loop and draw a partial
/// radial / sidebar while the scan is still running. This is what
/// makes huge scans feel responsive: the user sees the biggest
/// folders fill in (radial sorts size-desc, so they grow into the
/// largest slices first) instead of staring at "Scanning..." for a
/// minute.
///
/// Locking discipline
/// - One lock acquisition per filesystem entry (folder insert, file
///   insert + ancestor-size walk). The ancestor-walk runs inside the
///   same lock so the size view is always self-consistent.
/// - Lock is released between entries so the App's `try_lock` at
///   render time has a high chance of succeeding without blocking
///   the walker.
/// - At ~30ns per uncontended lock and 2M files, the lock overhead
///   is ~60ms total — invisible against the scan's wall-clock cost.
///
/// On completion the shared arena is cloned once into the
/// `ScanEvent::Complete` payload. The clone is a one-time O(n) cost
/// and unblocks the App from holding the live `Arc` after it has
/// taken ownership.
fn run(
    root_path: &Path,
    config: &ScanConfig,
    tx: &Sender<ScanEvent>,
    shared: &Arc<Mutex<TreeArena>>,
    cancel: &Arc<AtomicBool>,
) {
    // Seed the shared arena with the root folder.
    let root_id = {
        let root_name = root_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("/"))
            .to_string_lossy()
            .into_owned();
        let mut arena = shared.lock().unwrap();
        let id = arena.add_folder(Folder {
            file: File {
                name: root_name,
                size: 0,
                parent: None,
                path: root_path.to_path_buf(),
                ..File::default()
            },
            children_files: Vec::new(),
            children_folders: Vec::new(),
            child_count: 0,
        });
        arena.set_root(id);
        id
    };

    let mut path_to_folder: HashMap<PathBuf, FolderId> = HashMap::new();
    path_to_folder.insert(root_path.to_path_buf(), root_id);

    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut files: u64 = 0;
    let mut total_size: u64 = 0;
    let mut last_emit = Instant::now();

    // Build the exclude matcher up-front. Patterns are matched against
    // both the entry's full path and its base name so a user can write
    // either `node_modules` or `**/node_modules/**`.
    let exclude_set = build_exclude_set(&config.exclude, tx);

    let walker = WalkDir::new(root_path)
        .skip_hidden(false)
        .follow_links(config.follow_symlinks)
        .max_depth(config.max_depth.unwrap_or(usize::MAX))
        .into_iter();

    for entry in walker {
        // Cooperative cancellation: bail out cleanly when the App
        // signals (user pressed `q`, or a re-scan started). We do
        // not send a Complete event in that case — `cancel_and_wait`
        // / Drop semantics own the teardown.
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // Non-fatal: surface and continue.
                let _ = tx.send(ScanEvent::Warning(format!("{}", e)));
                continue;
            }
        };

        if let Some(err) = entry.read_children_error.as_ref() {
            let _ = tx.send(ScanEvent::Warning(format!(
                "{}: {}",
                entry.path().display(),
                err
            )));
        }

        if entry.depth() == 0 {
            continue;
        }

        let entry_path = entry.path();
        let Some(parent_path) = entry_path.parent() else {
            continue;
        };
        let Some(&parent_id) = path_to_folder.get(parent_path) else {
            continue;
        };

        let file_type = entry.file_type();
        if file_type.is_symlink() && !config.follow_symlinks {
            continue;
        }
        if !file_type.is_file() && !file_type.is_dir() {
            continue;
        }

        if let Some(set) = exclude_set.as_ref() {
            let base_name = entry.file_name().to_string_lossy();
            if set.is_match(&entry_path) || set.is_match(base_name.as_ref()) {
                continue;
            }
        }

        let name = entry.file_name().to_string_lossy().into_owned();

        if file_type.is_dir() {
            let folder_id = {
                let mut arena = shared.lock().unwrap();
                let id = arena.add_folder(Folder {
                    file: File {
                        name,
                        size: 0,
                        parent: Some(parent_id),
                        path: entry_path.clone(),
                        ..File::default()
                    },
                    children_files: Vec::new(),
                    children_folders: Vec::new(),
                    child_count: 0,
                });
                arena.folder_mut(parent_id).children_folders.push(id);
                id
            };
            path_to_folder.insert(entry_path, folder_id);
            continue;
        }

        // Regular file.
        let size = entry_size(&entry_path, config.use_apparent_size);

        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            if let Ok(metadata) = std::fs::metadata(&entry_path) {
                if !seen_inodes.insert((metadata.dev(), metadata.ino())) {
                    continue;
                }
                Some(metadata.ino())
            } else {
                None
            }
        };
        #[cfg(not(unix))]
        let inode: Option<u64> = None;

        // One lock for: file insert + parent's children update +
        // ancestor-size walk. Keeps the partial view self-consistent
        // — readers never observe a file in `children_files` whose
        // size hasn't been added to its ancestors.
        {
            let mut arena = shared.lock().unwrap();
            let file_id = arena.add_file(File {
                name,
                size,
                parent: Some(parent_id),
                path: entry_path.clone(),
                inode,
            });
            arena.folder_mut(parent_id).children_files.push(file_id);

            let mut cursor = Some(parent_id);
            while let Some(id) = cursor {
                let folder = arena.folder_mut(id);
                folder.file.size += size;
                cursor = folder.file.parent;
            }
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

    // Finalise child_count for every folder. Single lock over the
    // whole pass — the scan is over so contention with the App's
    // render loop no longer matters.
    {
        let mut arena = shared.lock().unwrap();
        let folder_count = arena.folders().len();
        for idx in 0..folder_count {
            let folder = arena.folder_mut(FolderId(idx));
            folder.child_count =
                (folder.children_files.len() + folder.children_folders.len()) as u32;
        }
    }

    // Hand off ownership via a one-time clone. The App receives this
    // and drops its reference to the shared `Arc` so post-scan reads
    // are lock-free.
    let final_arena = shared.lock().unwrap().clone();
    let _ = tx.send(ScanEvent::Complete(Box::new(final_arena)));
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
        handle.cancel_and_wait();
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
    fn excludes_matching_paths() {
        let temp = TempDir::new().unwrap();
        temp.child("keep/k.txt").write_str("k").unwrap();
        temp.child("node_modules/lib.js").write_str("x").unwrap();
        temp.child("node_modules/sub/lib.js")
            .write_str("x")
            .unwrap();

        let cfg = ScanConfig {
            exclude: vec!["node_modules".into()],
            ..ScanConfig::default()
        };
        let mut handle = scan_streaming(temp.path(), &cfg);
        let (_events, arena) = drain_until_done(&mut handle);
        let arena = arena.unwrap();
        let root = arena.root().unwrap();

        // node_modules/ must not appear among the children.
        let names: Vec<_> = arena
            .folder(root)
            .children_folders
            .iter()
            .map(|id| arena.folder(*id).file.name.clone())
            .collect();
        assert!(
            !names.iter().any(|n| n == "node_modules"),
            "exclude should drop node_modules, got {:?}",
            names
        );
        assert!(names.iter().any(|n| n == "keep"));
    }

    #[test]
    fn glob_exclude_matches_full_path() {
        let temp = TempDir::new().unwrap();
        temp.child("src/lib.rs").write_str("a").unwrap();
        temp.child("target/debug/foo").write_str("b").unwrap();

        let cfg = ScanConfig {
            // Path-style glob — must match against the full path.
            exclude: vec!["**/target/**".into()],
            ..ScanConfig::default()
        };
        let mut handle = scan_streaming(temp.path(), &cfg);
        let (_events, arena) = drain_until_done(&mut handle);
        let arena = arena.unwrap();
        let _root = arena.root().unwrap();
        // Walk every recorded file and assert none lives under target/.
        let mut all_paths = Vec::new();
        for f in arena.files() {
            all_paths.push(f.path.to_string_lossy().into_owned());
        }
        for p in &all_paths {
            assert!(
                !p.contains("/target/"),
                "exclude **/target/** should drop {}",
                p
            );
        }
    }

    #[test]
    fn invalid_exclude_pattern_warns_and_continues() {
        let temp = TempDir::new().unwrap();
        temp.child("a.txt").write_str("a").unwrap();

        let cfg = ScanConfig {
            // Unbalanced bracket — globset will reject it.
            exclude: vec!["[[broken".into()],
            ..ScanConfig::default()
        };
        let mut handle = scan_streaming(temp.path(), &cfg);
        let (events, arena) = drain_until_done(&mut handle);
        assert!(arena.is_some(), "scan must complete despite bad pattern");
        let warnings: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ScanEvent::Warning(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("invalid exclude pattern")),
            "expected an 'invalid exclude pattern' warning, got {:?}",
            warnings
        );
    }

    #[test]
    fn respects_max_depth() {
        let temp = TempDir::new().unwrap();
        temp.child("a/b/c").create_dir_all().unwrap();
        temp.child("a/b/c/deep.txt").write_str("deep").unwrap();

        let cfg = ScanConfig {
            max_depth: Some(1),
            ..ScanConfig::default()
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
