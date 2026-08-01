use crate::app::DeleteProgressUpdate;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use walkdir::WalkDir;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Get actual disk usage for a file (handles sparse files correctly)
#[cfg(unix)]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

pub struct DeleteResult {
    pub total_bytes: u64,
    pub total_files: u64,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy)]
enum EntryKind {
    FileOrSymlink,
    Directory,
}

/// Delete a file, symlink, or directory with optional progress updates sent to UI
pub fn delete_directory(
    path: &PathBuf,
    progress_tx: Option<mpsc::Sender<DeleteProgressUpdate>>,
) -> Result<DeleteResult, Box<dyn std::error::Error>> {
    let root_meta = fs::symlink_metadata(path)?;
    let root_ft = root_meta.file_type();

    // Single file or symlink: remove directly (never call remove_dir on a file)
    if root_ft.is_file() || root_ft.is_symlink() {
        let size = disk_usage(&root_meta);
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(DeleteProgressUpdate::Progress {
                bytes_done: 0,
                bytes_total: size,
                files_done: 0,
                files_total: 1,
                current_file: path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("<unknown>")
                    .to_string(),
            });
        }

        match fs::remove_file(path) {
            Ok(()) => {
                if let Some(ref tx) = progress_tx {
                    let _ = tx.send(DeleteProgressUpdate::Progress {
                        bytes_done: size,
                        bytes_total: size,
                        files_done: 1,
                        files_total: 1,
                        current_file: "Complete".to_string(),
                    });
                }
                Ok(DeleteResult {
                    total_bytes: size,
                    total_files: 1,
                    errors: Vec::new(),
                })
            }
            Err(e) => Ok(DeleteResult {
                total_bytes: 0,
                total_files: 0,
                errors: vec![format!("Failed to delete {}: {}", path.display(), e)],
            }),
        }
    } else if root_ft.is_dir() {
        delete_dir_tree(path, progress_tx)
    } else {
        Ok(DeleteResult {
            total_bytes: 0,
            total_files: 0,
            errors: vec![format!("Unsupported file type: {}", path.display())],
        })
    }
}

fn delete_dir_tree(
    path: &Path,
    progress_tx: Option<mpsc::Sender<DeleteProgressUpdate>>,
) -> Result<DeleteResult, Box<dyn std::error::Error>> {
    let mut total_bytes = 0u64;
    let mut total_files = 0u64;
    let mut errors = Vec::new();

    struct EntryWithMetadata {
        path: PathBuf,
        size: u64,
        kind: EntryKind,
    }

    // Collect with symlink-aware typing (do not follow links)
    let entries: Vec<EntryWithMetadata> = WalkDir::new(path)
        .same_file_system(true)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let entry_path = entry.path();
            if entry_path == path {
                return None;
            }

            let ft = entry.file_type();
            let kind = if ft.is_symlink() || ft.is_file() {
                EntryKind::FileOrSymlink
            } else if ft.is_dir() {
                EntryKind::Directory
            } else {
                // Treat other types (fifo, socket, …) as file-like for removal
                EntryKind::FileOrSymlink
            };

            let size = entry
                .metadata()
                .ok()
                .map(|m| disk_usage(&m))
                .or_else(|| {
                    fs::symlink_metadata(entry_path)
                        .ok()
                        .map(|m| disk_usage(&m))
                })
                .unwrap_or(0);

            Some(EntryWithMetadata {
                path: entry_path.to_path_buf(),
                size,
                kind,
            })
        })
        .collect();

    let total_count = entries.len() as u64;
    let total_size_bytes: u64 = entries.iter().map(|e| e.size).sum();

    if let Some(ref tx) = progress_tx {
        let _ = tx.send(DeleteProgressUpdate::Progress {
            bytes_done: 0,
            bytes_total: total_size_bytes,
            files_done: 0,
            files_total: total_count + 1,
            current_file: format!(
                "Found {} files, {:.1} MB",
                total_count,
                total_size_bytes as f64 / 1_048_576.0
            ),
        });
    }

    let mut files_deleted = 0u64;
    let mut bytes_deleted = 0u64;
    let progress_interval = std::cmp::max(1, total_count / 20);

    for (idx, entry) in entries.iter().rev().enumerate() {
        let ok = match entry.kind {
            EntryKind::FileOrSymlink => fs::remove_file(&entry.path).is_ok(),
            EntryKind::Directory => fs::remove_dir(&entry.path).is_ok(),
        };

        if ok {
            total_files += 1;
            if matches!(entry.kind, EntryKind::FileOrSymlink) {
                total_bytes += entry.size;
                files_deleted += 1;
                bytes_deleted += entry.size;
            }
        } else {
            errors.push(format!("Failed to delete {}", entry.path.display()));
        }

        if idx % progress_interval as usize == 0 || idx == entries.len().saturating_sub(1) {
            if let Some(ref tx) = progress_tx {
                if tx
                    .send(DeleteProgressUpdate::Progress {
                        bytes_done: bytes_deleted,
                        bytes_total: total_size_bytes,
                        files_done: files_deleted,
                        files_total: total_count + 1,
                        current_file: entry
                            .path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("<unknown>")
                            .to_string(),
                    })
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    // Remove root directory
    if let Err(e) = fs::remove_dir(path) {
        errors.push(format!("Failed to remove root directory: {}", e));
    } else {
        total_files += 1;
    }

    if let Some(ref tx) = progress_tx {
        let _ = tx.send(DeleteProgressUpdate::Progress {
            bytes_done: total_bytes,
            bytes_total: total_size_bytes,
            files_done: total_files,
            files_total: total_count + 1,
            current_file: "Complete".to_string(),
        });
    }

    Ok(DeleteResult {
        total_bytes,
        total_files,
        errors,
    })
}

pub fn dry_run_delete(path: &PathBuf) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_file() || meta.file_type().is_symlink() {
        return Ok(vec![path.clone()]);
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(path)
        .same_file_system(true)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        files.push(entry.path().to_path_buf());
    }

    files.push(path.clone());
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn deletes_single_file() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("solo.txt");
        fs::write(&file, "hello").unwrap();

        let result = delete_directory(&file, None).unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(result.total_files, 1);
        assert!(!file.exists());
    }

    #[test]
    fn deletes_symlink_without_touching_target() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("target.txt");
        fs::write(&target, "keep").unwrap();
        let link = tmp.path().join("link.txt");
        symlink(&target, &link).unwrap();

        let result = delete_directory(&link, None).unwrap();
        assert!(result.errors.is_empty());
        assert!(!link.exists());
        assert!(target.exists());
    }

    #[test]
    fn deletes_dir_containing_symlinks() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("tree");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        let dangling = root.join("dangling");
        symlink("/nonexistent/target", &dangling).unwrap();
        let to_dir = root.join("to_sub");
        symlink(root.join("sub"), &to_dir).unwrap();

        let result = delete_directory(&root, None).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(!root.exists());
    }
}
