use std::path::PathBuf;
use std::fs;
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    #[allow(dead_code)]
    pub file_count: u64,
    pub size_change: Option<(i64, f32)>, // (delta_bytes, delta_percent)
    #[allow(dead_code)]
    pub is_new: bool, // True if this didn't exist before
}

pub fn scan_directory(path: &PathBuf) -> Result<Vec<DirEntry>, Box<dyn std::error::Error>> {
    // Scan immediate children (non-recursive)
    let children: Vec<_> = fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .collect();

    // Process sequentially - directory size calculation is I/O bound and parallel doesn't help much
    // Plus we need to keep responsiveness
    let entries: Vec<DirEntry> = children
        .into_iter()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            let is_dir = metadata.is_dir();

            let name = path
                .file_name()?
                .to_str()?
                .to_string();

            // Fast size calculation
            let size = if is_dir {
                quick_dir_size(&path)
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
    // Ultra-fast size calculation for UI responsiveness
    // Only counts first 50000 files to keep latency low
    WalkDir::new(path)
        .into_iter()
        .take(50000) // Hard limit - prevents huge traversals from blocking UI
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}
