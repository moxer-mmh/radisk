use std::path::PathBuf;

/// Unique identifier for files in the arena
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId(pub usize);

/// Unique identifier for folders in the arena
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FolderId(pub usize);

/// A file entry in the tree
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct File {
    pub name: String,
    pub size: u64,
    pub parent: Option<FolderId>,
    pub path: PathBuf,
}

/// A folder entry in the tree
#[derive(Debug, Clone)]
pub struct Folder {
    pub file: File,
    pub children_files: Vec<FileId>,
    pub children_folders: Vec<FolderId>,
    pub child_count: u32,
}

/// Arena allocator for the file tree
#[derive(Debug)]
pub struct TreeArena {
    files: Vec<File>,
    folders: Vec<Folder>,
    root: Option<FolderId>,
}

impl TreeArena {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            folders: Vec::new(),
            root: None,
        }
    }

    pub fn add_file(&mut self, file: File) -> FileId {
        let id = FileId(self.files.len());
        self.files.push(file);
        id
    }

    pub fn add_folder(&mut self, folder: Folder) -> FolderId {
        let id = FolderId(self.folders.len());
        self.folders.push(folder);
        id
    }

    pub fn set_root(&mut self, id: FolderId) {
        self.root = Some(id);
    }

    pub fn root(&self) -> Option<FolderId> {
        self.root
    }

    pub fn file(&self, id: FileId) -> &File {
        &self.files[id.0]
    }

    pub fn folder(&self, id: FolderId) -> &Folder {
        &self.folders[id.0]
    }

    pub fn folder_mut(&mut self, id: FolderId) -> &mut Folder {
        &mut self.folders[id.0]
    }

    #[allow(dead_code)]
    pub fn files(&self) -> &[File] {
        &self.files
    }

    #[allow(dead_code)]
    pub fn folders(&self) -> &[Folder] {
        &self.folders
    }

    /// Get all items (files and folders) of a folder, sorted by size descending
    pub fn folder_items(&self, folder_id: FolderId) -> Vec<TreeItem> {
        let folder = &self.folders[folder_id.0];
        let mut items = Vec::new();

        for &fid in &folder.children_files {
            items.push(TreeItem::File(fid, self.files[fid.0].size));
        }

        for &fid in &folder.children_folders {
            items.push(TreeItem::Folder(fid, self.folders[fid.0].file.size));
        }

        items.sort_by(|a, b| b.size().cmp(&a.size()));
        items
    }

    /// Get total file count in a folder (recursive)
    pub fn total_file_count(&self, folder_id: FolderId) -> u64 {
        let folder = &self.folders[folder_id.0];
        let mut count = folder.children_files.len() as u64;

        for &child_id in &folder.children_folders {
            count += self.total_file_count(child_id);
        }

        count
    }

    /// Create a simple test tree for testing
    #[cfg(test)]
    pub fn create_test_tree() -> Self {
        let mut arena = Self::new();

        let root_path = PathBuf::from("/test");
        let root_file = File {
            name: "test".to_string(),
            size: 0,
            parent: None,
            path: root_path.clone(),
        };
        let root_folder = Folder {
            file: root_file,
            children_files: Vec::new(),
            children_folders: Vec::new(),
            child_count: 0,
        };
        let root_id = arena.add_folder(root_folder);
        arena.set_root(root_id);

        // Add files
        let f1 = File {
            name: "big.txt".to_string(),
            size: 1000,
            parent: Some(root_id),
            path: root_path.join("big.txt"),
        };
        let f1_id = arena.add_file(f1);

        let f2 = File {
            name: "small.txt".to_string(),
            size: 100,
            parent: Some(root_id),
            path: root_path.join("small.txt"),
        };
        let f2_id = arena.add_file(f2);

        // Add subfolder
        let sub_path = root_path.join("subdir");
        let sub_file = File {
            name: "subdir".to_string(),
            size: 500,
            parent: Some(root_id),
            path: sub_path.clone(),
        };
        let sub_folder = Folder {
            file: sub_file,
            children_files: Vec::new(),
            children_folders: Vec::new(),
            child_count: 0,
        };
        let sub_id = arena.add_folder(sub_folder);

        let f3 = File {
            name: "nested.txt".to_string(),
            size: 500,
            parent: Some(sub_id),
            path: sub_path.join("nested.txt"),
        };
        let f3_id = arena.add_file(f3);

        // Wire up children
        let root = arena.folder_mut(root_id);
        root.children_files.push(f1_id);
        root.children_files.push(f2_id);
        root.children_folders.push(sub_id);
        root.file.size = 1600;
        root.child_count = 3;

        let sub = arena.folder_mut(sub_id);
        sub.children_files.push(f3_id);
        sub.child_count = 1;

        arena
    }
}

impl Default for TreeArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents either a file or folder in tree traversal
#[derive(Debug, Clone, Copy)]
pub enum TreeItem {
    File(FileId, u64),
    Folder(FolderId, u64),
}

impl TreeItem {
    pub fn size(&self) -> u64 {
        match self {
            TreeItem::File(_, s) | TreeItem::Folder(_, s) => *s,
        }
    }

    pub fn is_folder(&self) -> bool {
        matches!(self, TreeItem::Folder(..))
    }
}

/// Format file size to human-readable string
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if size >= TB {
        format!("{:.1} TB", size as f64 / TB as f64)
    } else if size >= GB {
        format!("{:.1} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.1} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.1} KB", size as f64 / KB as f64)
    } else {
        format!("{} B", size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_creation() {
        let file = File {
            name: "test.txt".to_string(),
            size: 1024,
            parent: None,
            path: PathBuf::from("/test.txt"),
        };
        assert_eq!(file.name, "test.txt");
        assert_eq!(file.size, 1024);
    }

    #[test]
    fn test_folder_creation() {
        let file = File {
            name: "mydir".to_string(),
            size: 0,
            parent: None,
            path: PathBuf::from("/mydir"),
        };
        let folder = Folder {
            file,
            children_files: Vec::new(),
            children_folders: Vec::new(),
            child_count: 0,
        };
        assert_eq!(folder.file.name, "mydir");
        assert_eq!(folder.children_files.len(), 0);
        assert_eq!(folder.child_count, 0);
    }

    #[test]
    fn test_arena_add_and_retrieve() {
        let mut arena = TreeArena::new();

        let file = File {
            name: "a.txt".to_string(),
            size: 100,
            parent: None,
            path: PathBuf::from("/a.txt"),
        };
        let fid = arena.add_file(file);
        assert_eq!(fid, FileId(0));
        assert_eq!(arena.file(fid).name, "a.txt");
        assert_eq!(arena.file(fid).size, 100);
    }

    #[test]
    fn test_size_accumulation() {
        let arena = TreeArena::create_test_tree();
        let root_id = arena.root().unwrap();
        let root = arena.folder(root_id);

        // Root should have accumulated all sizes: 1000 + 100 + 500 = 1600
        assert_eq!(root.file.size, 1600);
        assert_eq!(root.child_count, 3);
    }

    #[test]
    fn test_children_sorted_by_size() {
        let arena = TreeArena::create_test_tree();
        let root_id = arena.root().unwrap();
        let items = arena.folder_items(root_id);

        assert_eq!(items.len(), 3);
        // Should be sorted descending: big.txt(1000), subdir(500), small.txt(100)
        assert_eq!(items[0].size(), 1000);
        assert_eq!(items[1].size(), 500);
        assert_eq!(items[2].size(), 100);
    }

    #[test]
    fn test_folder_items_contains_both_types() {
        let arena = TreeArena::create_test_tree();
        let root_id = arena.root().unwrap();
        let items = arena.folder_items(root_id);

        let file_count = items.iter().filter(|i| !i.is_folder()).count();
        let folder_count = items.iter().filter(|i| i.is_folder()).count();

        assert_eq!(file_count, 2);
        assert_eq!(folder_count, 1);
    }

    #[test]
    fn test_empty_folder_items() {
        let mut arena = TreeArena::new();

        let file = File {
            name: "empty".to_string(),
            size: 0,
            parent: None,
            path: PathBuf::from("/empty"),
        };
        let folder = Folder {
            file,
            children_files: Vec::new(),
            children_folders: Vec::new(),
            child_count: 0,
        };
        let id = arena.add_folder(folder);

        let items = arena.folder_items(id);
        assert!(items.is_empty());
    }

    #[test]
    fn test_tree_depth() {
        let arena = TreeArena::create_test_tree();
        let root_id = arena.root().unwrap();
        let sub_id = arena.folder(root_id).children_folders[0];

        // Subfolder has 1 file
        let sub_items = arena.folder_items(sub_id);
        assert_eq!(sub_items.len(), 1);
        assert_eq!(sub_items[0].size(), 500);
    }

    #[test]
    fn test_total_file_count() {
        let arena = TreeArena::create_test_tree();
        let root_id = arena.root().unwrap();

        let count = arena.total_file_count(root_id);
        assert_eq!(count, 3); // big.txt, small.txt, nested.txt
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(1073741824), "1.0 GB");
        assert_eq!(format_size(1099511627776), "1.0 TB");
    }

    #[test]
    fn test_folder_items_preserves_parent_reference() {
        let arena = TreeArena::create_test_tree();
        let root_id = arena.root().unwrap();

        // Check that files reference the root as parent
        let root = arena.folder(root_id);
        for &fid in &root.children_files {
            assert_eq!(arena.file(fid).parent, Some(root_id));
        }
    }
}
