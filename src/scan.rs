use crate::cache::SizeCache;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    #[allow(dead_code)]
    pub file_count: u64,
    pub size_change: Option<(i64, f32)>, // (delta_bytes, percent_of_directory)
    #[allow(dead_code)]
    pub is_new: bool, // True if this didn't exist before
}

pub fn scan_directory(
    path: &PathBuf,
    cache: &SizeCache,
    progress_tx: Option<&mpsc::Sender<crate::app::ScanResult>>,
) -> Result<Vec<DirEntry>, Box<dyn std::error::Error>> {
    // Scan immediate children (non-recursive)
    let children: Vec<_> = fs::read_dir(path)?.filter_map(|e| e.ok()).collect();

    let total_count = children.len();
    let mut scanned_count = 0;

    // Process sequentially - directory size calculation is I/O bound and parallel doesn't help much
    // Plus we need to keep responsiveness
    let entries: Vec<DirEntry> = children
        .into_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            let is_dir = metadata.is_dir();

            let name = path.file_name()?.to_str()?.to_string();

            // Fast size calculation - reuse metadata for files, use cache for dirs
            let size = if is_dir {
                // Send progress update for directories (skip files since they're fast)
                if let Some(tx) = progress_tx {
                    scanned_count += 1;
                    let _ = tx.send(crate::app::ScanResult::Progress {
                        current_name: name.clone(),
                        scanned_count,
                        total_count,
                    });
                }
                // Try cache first, fall back to scanning
                if let Some(cached_size) = cache.get(&path) {
                    cached_size
                } else {
                    let size = quick_dir_size(&path);
                    cache.set(path.clone(), size);
                    size
                }
            } else {
                metadata.len()
            };

            Some(DirEntry {
                path,
                name,
                size,
                is_dir,
                file_count: 0,
                size_change: None,
                is_new: false,
            })
        })
        .collect();

    // Sort by size, largest first
    let mut sorted = entries;
    sorted.sort_by(|a, b| b.size.cmp(&a.size));

    Ok(sorted)
}

fn quick_dir_size(path: &std::path::Path) -> u64 {
    // Optimized size calculation with early termination for huge directories
    // Uses DirEntry's metadata when available to avoid double stat calls
    let mut total = 0u64;
    const MAX_FILES: usize = 100_000; // Increased from 50k for better accuracy

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .take(MAX_FILES)
    {
        // Get file size from metadata already loaded by WalkDir
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                total += metadata.len();
            }
        }
    }

    total
}
