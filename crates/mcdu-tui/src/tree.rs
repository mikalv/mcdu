use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug)]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    /// Files are always complete; directories become complete after their subtree is scanned.
    pub complete: bool,
    pub children: Vec<FileNode>,
}

/// Progress update during tree scanning
pub enum ScanProgress {
    Scanning {
        files_scanned: usize,
        current_path: String,
    },
    /// Immediate children of `path` after readdir (dirs may still be incomplete).
    Listed {
        path: PathBuf,
        children: Vec<FileNode>,
    },
    /// Size update for `path` (partial while `complete` is false).
    Sized {
        path: PathBuf,
        size: u64,
        complete: bool,
    },
    Complete,
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

#[cfg(unix)]
fn device_id(metadata: &std::fs::Metadata) -> u64 {
    metadata.dev()
}

#[cfg(not(unix))]
fn device_id(_metadata: &std::fs::Metadata) -> u64 {
    0
}

fn cancelled(cancel: &Option<Arc<AtomicBool>>) -> bool {
    cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
}

fn send_progress(tx: &Option<mpsc::SyncSender<ScanProgress>>, msg: ScanProgress) {
    if let Some(tx) = tx {
        match &msg {
            // Drop noisy counter updates if the UI is behind
            ScanProgress::Scanning { .. } => {
                let _ = tx.try_send(msg);
            }
            // Listed/Sized/Complete apply backpressure so the UI stays responsive
            _ => {
                let _ = tx.send(msg);
            }
        }
    }
}

impl FileNode {
    pub fn new_file(name: String, path: PathBuf, size: u64) -> Self {
        FileNode {
            name,
            path,
            size,
            is_dir: false,
            complete: true,
            children: Vec::new(),
        }
    }

    pub fn new_dir(name: String, path: PathBuf) -> Self {
        FileNode {
            name,
            path,
            size: 0,
            is_dir: true,
            complete: false,
            children: Vec::new(),
        }
    }

    /// Sort children by size (largest first), then name for stability
    pub fn sort_children_by_size(&mut self) {
        self.children
            .sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    }

    /// Sort children recursively by size (largest first)
    pub fn sort_children(&mut self) {
        self.sort_children_by_size();
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
            self.complete = self.children.iter().all(|c| c.complete);
        }
        self.size
    }
}

/// Scan entire directory tree and build in-memory structure.
/// If `cancel` is set, returns an error early so the UI can start a new scan.
pub fn scan_tree(
    root: &Path,
    progress_tx: Option<mpsc::SyncSender<ScanProgress>>,
) -> Result<FileNode, String> {
    scan_tree_cancellable(root, progress_tx, None)
}

pub fn scan_tree_cancellable(
    root: &Path,
    progress_tx: Option<mpsc::SyncSender<ScanProgress>>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<FileNode, String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let root_meta = fs::metadata(&root).map_err(|e| e.to_string())?;
    if !root_meta.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    let root_dev = device_id(&root_meta);

    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.display().to_string());

    let mut root_node = FileNode::new_dir(name, root.clone());
    let mut files_scanned = 0usize;

    let size = scan_dir_recursive(
        &mut root_node,
        root_dev,
        &progress_tx,
        &cancel,
        &mut files_scanned,
    )?;

    root_node.size = size;
    root_node.complete = true;
    root_node.sort_children();

    send_progress(
        &progress_tx,
        ScanProgress::Sized {
            path: root,
            size,
            complete: true,
        },
    );
    send_progress(&progress_tx, ScanProgress::Complete);

    Ok(root_node)
}

/// Directory-first DFS: list children immediately, then descend.
fn scan_dir_recursive(
    node: &mut FileNode,
    root_dev: u64,
    progress_tx: &Option<mpsc::SyncSender<ScanProgress>>,
    cancel: &Option<Arc<AtomicBool>>,
    files_scanned: &mut usize,
) -> Result<u64, String> {
    if cancelled(cancel) {
        return Err("scan cancelled".to_string());
    }

    let read = match fs::read_dir(&node.path) {
        Ok(rd) => rd,
        Err(_) => {
            // Permission / vanished — treat as empty complete dir
            node.children.clear();
            node.size = 0;
            node.complete = true;
            send_progress(
                progress_tx,
                ScanProgress::Listed {
                    path: node.path.clone(),
                    children: Vec::new(),
                },
            );
            send_progress(
                progress_tx,
                ScanProgress::Sized {
                    path: node.path.clone(),
                    size: 0,
                    complete: true,
                },
            );
            return Ok(0);
        }
    };

    let mut children = Vec::new();
    for entry in read.flatten() {
        if cancelled(cancel) {
            return Err("scan cancelled".to_string());
        }

        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        // Skip mount crossings (same as WalkDir::same_file_system)
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if device_id(&meta) != root_dev {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        *files_scanned += 1;

        if (*files_scanned).is_multiple_of(1000) {
            send_progress(
                progress_tx,
                ScanProgress::Scanning {
                    files_scanned: *files_scanned,
                    current_path: path.display().to_string(),
                },
            );
        }

        if file_type.is_dir() {
            children.push(FileNode::new_dir(name, path));
        } else if file_type.is_file() {
            // Follow for sparse-aware disk usage on regular files
            let size = match fs::metadata(&path) {
                Ok(m) => disk_usage(&m),
                Err(_) => disk_usage(&meta),
            };
            children.push(FileNode::new_file(name, path, size));
        } else if file_type.is_symlink() {
            // Do not follow symlinks; count the link itself
            children.push(FileNode::new_file(name, path, disk_usage(&meta)));
        }
        // skip other special files
    }

    children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    node.children = children;

    send_progress(
        progress_tx,
        ScanProgress::Listed {
            path: node.path.clone(),
            children: node.children.clone(),
        },
    );

    // Initial size = sum of files already known
    let mut total: u64 = node
        .children
        .iter()
        .filter(|c| !c.is_dir)
        .map(|c| c.size)
        .sum();
    node.size = total;
    node.complete = false;

    send_progress(
        progress_tx,
        ScanProgress::Sized {
            path: node.path.clone(),
            size: total,
            complete: false,
        },
    );

    // Paths stay stable across re-sorts after each child finishes
    let dir_paths: Vec<PathBuf> = node
        .children
        .iter()
        .filter(|c| c.is_dir)
        .map(|c| c.path.clone())
        .collect();

    let dir_count = dir_paths.len();
    for (done, child_path) in dir_paths.into_iter().enumerate() {
        if cancelled(cancel) {
            return Err("scan cancelled".to_string());
        }

        let idx = node
            .children
            .iter()
            .position(|c| c.path == child_path)
            .ok_or_else(|| format!("missing child {}", child_path.display()))?;

        let child_size = {
            let child = &mut node.children[idx];
            scan_dir_recursive(child, root_dev, progress_tx, cancel, files_scanned)?
        };

        // Child already marked complete + Sized by recursion; update parent total
        total = total.saturating_add(child_size);
        node.size = total;
        node.sort_children_by_size();

        // Throttle parent size spam: every 8th child, plus last before final complete
        let n = done + 1;
        if n == dir_count || n % 8 == 0 {
            send_progress(
                progress_tx,
                ScanProgress::Sized {
                    path: node.path.clone(),
                    size: total,
                    complete: false,
                },
            );
        }
    }

    node.complete = true;
    node.size = total;
    node.sort_children_by_size();

    send_progress(
        progress_tx,
        ScanProgress::Sized {
            path: node.path.clone(),
            size: total,
            complete: true,
        },
    );

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn test_scan_tree_basic() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::create_dir(root.join("dir1")).unwrap();
        fs::write(root.join("dir1/file1.txt"), "hello").unwrap();
        fs::write(root.join("file2.txt"), "world").unwrap();

        let tree = scan_tree(root, None).unwrap();

        assert!(tree.is_dir);
        assert!(tree.complete);
        assert_eq!(tree.children.len(), 2);
        let dir1 = tree.children.iter().find(|c| c.name == "dir1").unwrap();
        assert!(dir1.is_dir);
        assert!(dir1.complete);
        assert_eq!(dir1.children.len(), 1);
    }

    #[test]
    fn test_incremental_listed_before_deep_complete() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::create_dir(root.join("deep")).unwrap();
        fs::create_dir(root.join("deep/nested")).unwrap();
        fs::write(root.join("deep/nested/file.txt"), "data").unwrap();
        fs::write(root.join("top.txt"), "x").unwrap();

        let (tx, rx) = mpsc::sync_channel(256);
        let root_canon = root.canonicalize().unwrap();
        let handle = std::thread::spawn(move || {
            scan_tree_cancellable(&root_canon, Some(tx), None).unwrap();
        });

        let mut saw_root_listed = false;
        let mut saw_complete = false;
        let mut events = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);

        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(msg) => {
                    match &msg {
                        ScanProgress::Listed { path, children }
                            if path == &root.canonicalize().unwrap() =>
                        {
                            saw_root_listed = true;
                            // Deep nested file must not be a direct child; listing is shallow
                            assert!(children.iter().any(|c| c.name == "deep"));
                            assert!(children.iter().any(|c| c.name == "top.txt"));
                            assert!(!children.iter().any(|c| c.name == "file.txt"));
                            // At least one dir should still be incomplete in the listed snapshot
                            let deep = children.iter().find(|c| c.name == "deep").unwrap();
                            assert!(!deep.complete);
                        }
                        ScanProgress::Complete => {
                            saw_complete = true;
                            events.push(msg);
                            break;
                        }
                        _ => {}
                    }
                    events.push(msg);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if handle.is_finished() && rx.try_recv().is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        handle.join().unwrap();
        assert!(saw_root_listed, "expected Listed for root before finish");
        assert!(saw_complete, "expected Complete");

        // First Listed for root should appear before Complete
        let root_listed_pos = events.iter().position(|e| {
            matches!(e, ScanProgress::Listed { path, .. } if path == &root.canonicalize().unwrap())
        });
        let complete_pos = events
            .iter()
            .position(|e| matches!(e, ScanProgress::Complete));
        assert!(root_listed_pos.unwrap() < complete_pos.unwrap());
    }

    #[test]
    fn test_size_rollup_and_complete_bottom_up() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::create_dir(root.join("a")).unwrap();
        fs::write(root.join("a/big.txt"), vec![0u8; 4096]).unwrap();
        fs::write(root.join("small.txt"), b"hi").unwrap();

        let (tx, rx) = mpsc::sync_channel(256);
        let root_canon = root.canonicalize().unwrap();
        let handle = std::thread::spawn({
            let root_canon = root_canon.clone();
            move || scan_tree_cancellable(&root_canon, Some(tx), None).unwrap()
        });

        let mut sized_events = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(ScanProgress::Sized {
                    path,
                    size,
                    complete,
                }) => sized_events.push((path, size, complete)),
                Ok(ScanProgress::Complete) => break,
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if handle.is_finished() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let tree = handle.join().unwrap();
        assert!(tree.complete);
        assert!(tree.size > 0);

        let a_path = root_canon.join("a");
        assert!(sized_events.iter().any(|(p, _, c)| p == &a_path && *c));

        let root_complete = sized_events
            .iter()
            .rev()
            .find(|(p, _, c)| p == &root_canon && *c);
        assert!(root_complete.is_some());
        assert_eq!(root_complete.unwrap().1, tree.size);
    }

    #[test]
    fn test_cancel_mid_scan() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Enough dirs that cancel can land mid-walk
        for i in 0..50 {
            let d = root.join(format!("d{i}"));
            fs::create_dir(&d).unwrap();
            fs::write(d.join("f.txt"), "x").unwrap();
        }

        let (tx, rx) = mpsc::sync_channel(256);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let root_canon = root.canonicalize().unwrap();

        // Cancel before the worker starts: the scan must abort before
        // completion regardless of scheduling speed. (Instrumented/tarpaulin
        // runs are too slow to reliably race a mid-walk cancel.)
        cancel.store(true, Ordering::Relaxed);
        let handle =
            std::thread::spawn(move || scan_tree_cancellable(&root_canon, Some(tx), Some(cancel_clone)));

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut saw_complete = false;
        let result = loop {
            while let Ok(msg) = rx.try_recv() {
                if matches!(msg, ScanProgress::Complete) {
                    saw_complete = true;
                }
            }
            if handle.is_finished() {
                break handle.join().unwrap();
            }
            if std::time::Instant::now() > deadline {
                panic!("timed out waiting for cancelled scan");
            }
            std::thread::sleep(Duration::from_millis(1));
        };

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "scan cancelled");
        assert!(!saw_complete, "cancelled scan must not send Complete");
    }
}
