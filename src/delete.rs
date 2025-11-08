use std::path::PathBuf;
use std::fs;
use walkdir::WalkDir;

pub struct DeleteResult {
    pub total_bytes: u64,
    pub total_files: u64,
    pub errors: Vec<String>,
}

pub fn delete_directory(path: &PathBuf) -> Result<DeleteResult, Box<dyn std::error::Error>> {
    let mut total_bytes = 0u64;
    let mut total_files = 0u64;
    let mut errors = Vec::new();

    // Pre-calculate total size for progress tracking
    let _total_size: u64 = WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    // Collect all entries in reverse order (deepest first)
    let entries: Vec<_> = WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    // Delete in reverse order (files first, then directories)
    for entry in entries.iter().rev() {
        let entry_path = entry.path();

        // Skip the root directory itself initially
        if entry_path == path {
            continue;
        }

        match entry.metadata() {
            Ok(metadata) => {
                if metadata.is_file() {
                    let size = metadata.len();
                    if fs::remove_file(entry_path).is_ok() {
                        total_bytes += size;
                        total_files += 1;
                    } else {
                        errors.push(format!("Failed to delete {}", entry_path.display()));
                    }
                } else if metadata.is_dir() {
                    if fs::remove_dir(entry_path).is_ok() {
                        total_files += 1;
                    } else {
                        errors.push(format!("Failed to delete {}", entry_path.display()));
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Failed to stat {}: {}", entry_path.display(), e));
            }
        }
    }

    // Finally, remove the root directory itself
    if let Err(e) = fs::remove_dir(path) {
        errors.push(format!("Failed to remove root directory: {}", e));
    } else {
        total_files += 1;
    }

    Ok(DeleteResult {
        total_bytes,
        total_files,
        errors,
    })
}

pub fn dry_run_delete(path: &PathBuf) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        files.push(entry.path().to_path_buf());
    }

    files.push(path.clone());
    Ok(files)
}
