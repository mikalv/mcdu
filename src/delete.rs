use std::fs;
use std::path::PathBuf;
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

    // Optimized: Single walk, collect entries with metadata to avoid re-stating
    struct EntryWithMetadata {
        path: PathBuf,
        size: u64,
        is_file: bool,
    }

    let entries: Vec<EntryWithMetadata> = WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let entry_path = entry.path();
            if entry_path == path {
                return None; // Skip root for now
            }

            // Get metadata once and store it
            entry.metadata().ok().map(|metadata| EntryWithMetadata {
                path: entry_path.to_path_buf(),
                size: metadata.len(),
                is_file: metadata.is_file(),
            })
        })
        .collect();

    // Delete in reverse order (files first, then directories)
    for entry in entries.iter().rev() {
        if entry.is_file {
            if fs::remove_file(&entry.path).is_ok() {
                total_bytes += entry.size;
                total_files += 1;
            } else {
                errors.push(format!("Failed to delete {}", entry.path.display()));
            }
        } else {
            if fs::remove_dir(&entry.path).is_ok() {
                total_files += 1;
            } else {
                errors.push(format!("Failed to delete {}", entry.path.display()));
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

    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        files.push(entry.path().to_path_buf());
    }

    files.push(path.clone());
    Ok(files)
}
