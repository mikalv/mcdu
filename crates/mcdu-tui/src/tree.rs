use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use walkdir::WalkDir;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
}

/// Progress update during tree scanning
pub enum ScanProgress {
    Scanning {
        files_scanned: usize,
        current_path: String,
    },
    Complete(FileNode),
    Error(String),
}

/// Get actual disk usage for a file (handles sparse files correctly)
#[cfg(unix)]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

impl FileNode {
    pub fn new_file(name: String, path: PathBuf, size: u64) -> Self {
        FileNode {
            name,
            path,
            size,
            is_dir: false,
            children: Vec::new(),
        }
    }

    pub fn new_dir(name: String, path: PathBuf) -> Self {
        FileNode {
            name,
            path,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        }
    }

    /// Sort children by size (largest first)
    pub fn sort_children(&mut self) {
        self.children.sort_by(|a, b| b.size.cmp(&a.size));
        for child in &mut self.children {
            if child.is_dir {
                child.sort_children();
            }
        }
    }

    /// Calculate size from children (for directories)
    pub fn calculate_size(&mut self) -> u64 {
        if self.is_dir {
            self.size = self.children.iter_mut().map(|c| c.calculate_size()).sum();
        }
        self.size
    }
}

/// Scan entire directory tree and build in-memory structure.
/// If `cancel` is set, returns an error early so the UI can start a new scan.
pub fn scan_tree(
    root: &Path,
    progress_tx: Option<mpsc::Sender<ScanProgress>>,
) -> Result<FileNode, String> {
    scan_tree_cancellable(root, progress_tx, None)
}

pub fn scan_tree_cancellable(
    root: &Path,
    progress_tx: Option<mpsc::Sender<ScanProgress>>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<FileNode, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;

    let mut entries: Vec<(PathBuf, u64, bool)> = Vec::new();
    let mut files_scanned = 0;

    for entry in WalkDir::new(&root)
        .same_file_system(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err("scan cancelled".to_string());
        }

        let path = entry.path().to_path_buf();

        if let Ok(metadata) = entry.metadata() {
            let size = if metadata.is_file() {
                disk_usage(&metadata)
            } else {
                0
            };

            entries.push((path.clone(), size, metadata.is_dir()));
            files_scanned += 1;

            if files_scanned % 1000 == 0 {
                if let Some(ref tx) = progress_tx {
                    let _ = tx.send(ScanProgress::Scanning {
                        files_scanned,
                        current_path: path.display().to_string(),
                    });
                }
            }
        }
    }

    let tree = build_tree(&root, entries);
    Ok(tree)
}

/// Build tree structure from flat list of entries
fn build_tree(root: &Path, entries: Vec<(PathBuf, u64, bool)>) -> FileNode {
    // Create a map of path -> children
    let mut children_map: HashMap<PathBuf, Vec<(PathBuf, u64, bool)>> = HashMap::new();

    for (path, size, is_dir) in entries {
        if path == root {
            continue; // Skip root, we'll create it separately
        }

        if let Some(parent) = path.parent() {
            children_map
                .entry(parent.to_path_buf())
                .or_default()
                .push((path, size, is_dir));
        }
    }

    // Recursively build the tree
    fn build_node(
        path: &Path,
        children_map: &HashMap<PathBuf, Vec<(PathBuf, u64, bool)>>,
    ) -> FileNode {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        let children_entries = children_map.get(path);

        if let Some(entries) = children_entries {
            // This is a directory with children
            let mut node = FileNode::new_dir(name, path.to_path_buf());

            for (child_path, size, is_dir) in entries {
                if *is_dir {
                    let child = build_node(child_path, children_map);
                    node.children.push(child);
                } else {
                    let child_name = child_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    node.children
                        .push(FileNode::new_file(child_name, child_path.clone(), *size));
                }
            }

            node
        } else {
            // Leaf directory (no children) or file
            FileNode::new_dir(name, path.to_path_buf())
        }
    }

    let mut tree = build_node(root, &children_map);
    tree.calculate_size();
    tree.sort_children();
    tree
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_scan_tree_basic() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Create test structure
        fs::create_dir(root.join("dir1")).unwrap();
        fs::write(root.join("dir1/file1.txt"), "hello").unwrap();
        fs::write(root.join("file2.txt"), "world").unwrap();

        let tree = scan_tree(root, None).unwrap();

        assert!(tree.is_dir);
        assert_eq!(tree.children.len(), 2);
    }
}
