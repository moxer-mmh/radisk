use crate::tree::{File, FileId, Folder, FolderId, TreeArena};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Configuration for the disk scanner
#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub follow_symlinks: bool,
    pub max_depth: Option<usize>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            max_depth: None,
        }
    }
}

/// Error type for scanning operations
#[derive(Debug)]
pub enum ScanError {
    IoError(io::Error),
    PermissionDenied(PathBuf),
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::IoError(e) => write!(f, "IO error: {}", e),
            ScanError::PermissionDenied(p) => write!(f, "Permission denied: {}", p.display()),
        }
    }
}

impl std::error::Error for ScanError {}

/// Progress information during scanning
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub files_scanned: u64,
    pub total_size: u64,
}

/// Get file size using st_blocks * 512 for accurate disk usage (Linux)
/// Falls back to file size on non-Linux platforms
fn get_file_size(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            // Use st_blocks * 512 for actual disk usage (matches FileLight)
            let blocks = metadata.blocks();
            if blocks > 0 {
                return blocks as u64 * 512;
            }
        }
    }
    // Fallback to regular file size
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Check if a path is a symlink
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Scan a directory and build a tree
pub fn scan_directory(path: &Path, config: &ScanConfig) -> Result<TreeArena, ScanError> {
    let mut arena = TreeArena::new();
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();

    // Create root folder
    let root_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("/"))
        .to_string_lossy()
        .into_owned();

    let root_folder = Folder {
        file: File {
            name: root_name,
            size: 0,
            parent: None,
            path: path.to_path_buf(),
        },
        children_files: Vec::new(),
        children_folders: Vec::new(),
        child_count: 0,
    };
    let root_id = arena.add_folder(root_folder);
    arena.set_root(root_id);

    // Scan recursively and set root size
    let total_size = scan_recursive(&mut arena, path, root_id, &mut seen_inodes, config, 0)?;
    arena.folder_mut(root_id).file.size = total_size;

    Ok(arena)
}

fn scan_recursive(
    arena: &mut TreeArena,
    dir_path: &Path,
    parent_id: FolderId,
    seen_inodes: &mut HashSet<(u64, u64)>,
    config: &ScanConfig,
    depth: usize,
) -> Result<u64, ScanError> {
    let mut total_size: u64 = 0;

    let read_dir = match std::fs::read_dir(dir_path) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Err(ScanError::PermissionDenied(dir_path.to_path_buf()));
        }
        Err(e) => return Err(ScanError::IoError(e)),
    };

    let mut child_items: Vec<(PathBuf, u64, bool)> = Vec::new();

    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        // Skip symlinks unless configured to follow
        if file_type.is_symlink() && !config.follow_symlinks {
            continue;
        }

        // Skip non-file, non-directory entries (devices, pipes, etc.)
        if !file_type.is_file() && !file_type.is_dir() {
            continue;
        }

        if file_type.is_file() {
            let size = get_file_size(&entry_path);

            // Hard link dedup (Linux)
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Ok(metadata) = std::fs::metadata(&entry_path) {
                    let dev = metadata.dev();
                    let ino = metadata.ino();
                    if !seen_inodes.insert((dev, ino)) {
                        // Duplicate hard link, skip
                        continue;
                    }
                }
            }

            child_items.push((entry_path.clone(), size, false));
            total_size += size;
        } else if file_type.is_dir() {
            // Check depth limit
            if let Some(max) = config.max_depth {
                if depth >= max {
                    continue;
                }
            }
            child_items.push((entry_path.clone(), 0, true));
        }
    }

    // Sort by size descending (largest first, as FileLight does)
    child_items.sort_by(|a, b| b.1.cmp(&a.1));

    for (item_path, item_size, is_dir) in child_items {
        let name = item_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        if is_dir {
            // Create subfolder
            let sub_folder = Folder {
                file: File {
                    name: name.clone(),
                    size: 0,
                    parent: Some(parent_id),
                    path: item_path.clone(),
                },
                children_files: Vec::new(),
                children_folders: Vec::new(),
                child_count: 0,
            };
            let sub_id = arena.add_folder(sub_folder);

            // Add to parent
            let parent = arena.folder_mut(parent_id);
            parent.children_folders.push(sub_id);

            // Recurse into subdirectory
            match scan_recursive(arena, &item_path, sub_id, seen_inodes, config, depth + 1) {
                Ok(sub_size) => {
                    let folder = arena.folder_mut(sub_id);
                    folder.file.size = sub_size;
                    total_size += sub_size;
                }
                Err(ScanError::PermissionDenied(_)) => {
                    // Skip directories we can't read, size stays 0
                }
                Err(e) => return Err(e),
            }
        } else {
            // Create file
            let file = File {
                name,
                size: item_size,
                parent: Some(parent_id),
                path: item_path.clone(),
            };
            let file_id = arena.add_file(file);

            // Add to parent
            let parent = arena.folder_mut(parent_id);
            parent.children_files.push(file_id);
        }
    }

    // Update parent's child count
    let parent = arena.folder_mut(parent_id);
    parent.child_count = (parent.children_files.len() + parent.children_folders.len()) as u32;

    Ok(total_size)
}

/// Scan with progress reporting (background thread)
pub fn scan_with_progress(
    path: &Path,
    config: &ScanConfig,
    progress_tx: Option<std::sync::mpsc::Sender<ScanProgress>>,
) -> Result<TreeArena, ScanError> {
    let mut arena = TreeArena::new();
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut progress = ScanProgress {
        files_scanned: 0,
        total_size: 0,
    };

    let root_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("/"))
        .to_string_lossy()
        .into_owned();

    let root_folder = Folder {
        file: File {
            name: root_name,
            size: 0,
            parent: None,
            path: path.to_path_buf(),
        },
        children_files: Vec::new(),
        children_folders: Vec::new(),
        child_count: 0,
    };
    let root_id = arena.add_folder(root_folder);
    arena.set_root(root_id);

    let total_size = scan_recursive_with_progress(
        &mut arena,
        path,
        root_id,
        &mut seen_inodes,
        config,
        0,
        &mut progress,
        &progress_tx,
    )?;
    arena.folder_mut(root_id).file.size = total_size;

    Ok(arena)
}

fn scan_recursive_with_progress(
    arena: &mut TreeArena,
    dir_path: &Path,
    parent_id: FolderId,
    seen_inodes: &mut HashSet<(u64, u64)>,
    config: &ScanConfig,
    depth: usize,
    progress: &mut ScanProgress,
    progress_tx: &Option<std::sync::mpsc::Sender<ScanProgress>>,
) -> Result<u64, ScanError> {
    let mut total_size: u64 = 0;

    let read_dir = match std::fs::read_dir(dir_path) {
        Ok(rd) => rd,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Err(ScanError::PermissionDenied(dir_path.to_path_buf()));
        }
        Err(e) => return Err(ScanError::IoError(e)),
    };

    let mut child_items: Vec<(PathBuf, u64, bool)> = Vec::new();

    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_symlink() && !config.follow_symlinks {
            continue;
        }

        if !file_type.is_file() && !file_type.is_dir() {
            continue;
        }

        if file_type.is_file() {
            let size = get_file_size(&entry_path);

            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Ok(metadata) = std::fs::metadata(&entry_path) {
                    let dev = metadata.dev();
                    let ino = metadata.ino();
                    if !seen_inodes.insert((dev, ino)) {
                        continue;
                    }
                }
            }

            child_items.push((entry_path.clone(), size, false));
            total_size += size;

            progress.files_scanned += 1;
            progress.total_size += size;

            // Report progress
            if let Some(tx) = progress_tx {
                let _ = tx.send(progress.clone());
            }
        } else if file_type.is_dir() {
            if let Some(max) = config.max_depth {
                if depth >= max {
                    continue;
                }
            }
            child_items.push((entry_path.clone(), 0, true));
            progress.files_scanned += 1;

            if let Some(tx) = progress_tx {
                let _ = tx.send(progress.clone());
            }
        }
    }

    child_items.sort_by(|a, b| b.1.cmp(&a.1));

    for (item_path, item_size, is_dir) in child_items {
        let name = item_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        if is_dir {
            let sub_folder = Folder {
                file: File {
                    name: name.clone(),
                    size: 0,
                    parent: Some(parent_id),
                    path: item_path.clone(),
                },
                children_files: Vec::new(),
                children_folders: Vec::new(),
                child_count: 0,
            };
            let sub_id = arena.add_folder(sub_folder);

            let parent = arena.folder_mut(parent_id);
            parent.children_folders.push(sub_id);

            match scan_recursive_with_progress(
                arena,
                &item_path,
                sub_id,
                seen_inodes,
                config,
                depth + 1,
                progress,
                progress_tx,
            ) {
                Ok(sub_size) => {
                    let folder = arena.folder_mut(sub_id);
                    folder.file.size = sub_size;
                    total_size += sub_size;
                }
                Err(ScanError::PermissionDenied(_)) => {}
                Err(e) => return Err(e),
            }
        } else {
            let file = File {
                name,
                size: item_size,
                parent: Some(parent_id),
                path: item_path.clone(),
            };
            let file_id = arena.add_file(file);

            let parent = arena.folder_mut(parent_id);
            parent.children_files.push(file_id);
        }
    }

    let parent = arena.folder_mut(parent_id);
    parent.child_count = (parent.children_files.len() + parent.children_folders.len()) as u32;

    Ok(total_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    fn create_test_fs() -> TempDir {
        let temp = TempDir::new().unwrap();

        temp.child("dir1").create_dir_all().unwrap();
        temp.child("dir1/file1.txt").write_str("hello").unwrap(); // 5 bytes
        temp.child("dir1/file2.txt").write_str("world!").unwrap(); // 6 bytes

        temp.child("dir2").create_dir_all().unwrap();
        temp.child("dir2/subdir").create_dir_all().unwrap();
        temp.child("dir2/subdir/file3.txt")
            .write_str("test content")
            .unwrap(); // 12 bytes

        temp.child("root_file.txt")
            .write_str("root level file")
            .unwrap(); // 15 bytes

        temp
    }

    #[test]
    fn test_scan_empty_dir() {
        let temp = TempDir::new().unwrap();
        temp.child("empty").create_dir_all().unwrap();

        let config = ScanConfig::default();
        let arena = scan_directory(temp.child("empty").path(), &config).unwrap();

        let root_id = arena.root().unwrap();
        let root = arena.folder(root_id);

        assert_eq!(root.children_files.len(), 0);
        assert_eq!(root.children_folders.len(), 0);
        assert_eq!(root.child_count, 0);
    }

    #[test]
    fn test_scan_with_files() {
        let temp = create_test_fs();
        let config = ScanConfig::default();
        let arena = scan_directory(temp.path(), &config).unwrap();

        let root_id = arena.root().unwrap();
        let root = arena.folder(root_id);

        // Should have 1 root file + 2 directories
        assert_eq!(root.children_files.len(), 1);
        assert_eq!(root.children_folders.len(), 2);
        assert!(root.file.size > 0);

        // Check root file
        let root_file = arena.file(root.children_files[0]);
        assert_eq!(root_file.name, "root_file.txt");
    }

    #[test]
    fn test_scan_nested_dirs() {
        let temp = create_test_fs();
        let config = ScanConfig::default();
        let arena = scan_directory(temp.path(), &config).unwrap();

        let root_id = arena.root().unwrap();
        let items = arena.folder_items(root_id);

        // Should have items sorted by size
        assert!(items.len() >= 3);

        // dir2 should have a subdir
        let dir2_id = arena
            .folder(root_id)
            .children_folders
            .iter()
            .find(|&&id| arena.folder(id).file.name == "dir2")
            .copied()
            .unwrap();

        let dir2 = arena.folder(dir2_id);
        assert!(dir2.children_folders.len() >= 1);
    }

    #[test]
    fn test_scan_skips_symlinks() {
        let temp = TempDir::new().unwrap();
        temp.child("real.txt").write_str("real content").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                temp.child("real.txt").path(),
                temp.child("link.txt").path(),
            )
            .unwrap();

            let config = ScanConfig {
                follow_symlinks: false,
                ..Default::default()
            };
            let arena = scan_directory(temp.path(), &config).unwrap();

            let root_id = arena.root().unwrap();
            let root = arena.folder(root_id);

            // Should only have the real file, not the symlink
            assert_eq!(root.children_files.len(), 1);
            assert_eq!(arena.file(root.children_files[0]).name, "real.txt");
        }
    }

    #[test]
    fn test_scan_size_calculation() {
        let temp = TempDir::new().unwrap();
        temp.child("test.txt").write_str("12345").unwrap(); // 5 bytes

        let config = ScanConfig::default();
        let arena = scan_directory(temp.path(), &config).unwrap();

        let root_id = arena.root().unwrap();
        let root = arena.folder(root_id);

        assert_eq!(root.children_files.len(), 1);
        // Size may be larger due to block allocation, but should be >= 5
        let file = arena.file(root.children_files[0]);
        assert!(file.size >= 5, "File size {} should be >= 5", file.size);
    }

    #[test]
    fn test_scan_permission_denied() {
        let temp = TempDir::new().unwrap();
        temp.child("restricted").create_dir_all().unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                temp.child("restricted").path(),
                std::fs::Permissions::from_mode(0o000),
            )
            .unwrap();

            let config = ScanConfig::default();
            // Should not panic - restricted dir gets skipped
            let result = scan_directory(temp.path(), &config);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_scan_progress_reporting() {
        let temp = create_test_fs();
        let config = ScanConfig::default();
        let (tx, rx) = std::sync::mpsc::channel();

        let _arena = scan_with_progress(temp.path(), &config, Some(tx)).unwrap();

        // Should have received progress updates
        let mut updates = Vec::new();
        while let Ok(progress) = rx.try_recv() {
            updates.push(progress);
        }

        assert!(!updates.is_empty());
        // Final update should show files scanned
        let final_progress = updates.last().unwrap();
        assert!(final_progress.files_scanned > 0);
        assert!(final_progress.total_size > 0);
    }

    #[test]
    fn test_scan_folder_sizes_accumulate() {
        let temp = create_test_fs();
        let config = ScanConfig::default();
        let arena = scan_directory(temp.path(), &config).unwrap();

        let root_id = arena.root().unwrap();
        let root = arena.folder(root_id);

        // Root size should equal sum of all files
        assert!(root.file.size > 0);

        // Each subfolder should have a size > 0
        for &dir_id in &root.children_folders {
            let dir = arena.folder(dir_id);
            assert!(
                dir.file.size > 0,
                "Directory {} should have size > 0",
                dir.file.name
            );
        }
    }

    #[test]
    fn test_scan_files_sorted_by_size() {
        let temp = TempDir::new().unwrap();
        temp.child("small.txt").write_str("a").unwrap();
        temp.child("medium.txt").write_str("aaaa").unwrap();
        temp.child("large.txt").write_str("aaaaaaa").unwrap();

        let config = ScanConfig::default();
        let arena = scan_directory(temp.path(), &config).unwrap();

        let root_id = arena.root().unwrap();
        let root = arena.folder(root_id);

        // Files should be sorted by size descending
        for i in 1..root.children_files.len() {
            let prev = arena.file(root.children_files[i - 1]).size;
            let curr = arena.file(root.children_files[i]).size;
            assert!(
                prev >= curr,
                "Files not sorted: {} ({}) before {} ({})",
                arena.file(root.children_files[i - 1]).name,
                prev,
                arena.file(root.children_files[i]).name,
                curr
            );
        }
    }
}
