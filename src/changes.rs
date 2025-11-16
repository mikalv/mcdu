use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Represents a snapshot of directory state using hashes instead of paths
#[derive(Debug, Clone)]
pub struct DirectoryFingerprint {
    /// Hash of (name + size) for each entry - compact bitmap-like representation
    pub entries: HashMap<String, (u64, u64)>, // name -> (size, mtime)
}

/// Delta between current and previous scan
#[derive(Debug, Clone)]
pub struct SizeChange {
    pub name: String,
    #[allow(dead_code)]
    pub old_size: u64,
    #[allow(dead_code)]
    pub new_size: u64,
    pub delta_bytes: i64,
    #[allow(dead_code)]
    pub delta_percent: f32,
}

impl DirectoryFingerprint {
    pub fn new() -> Self {
        DirectoryFingerprint {
            entries: HashMap::new(),
        }
    }

    /// Create fingerprint from current directory state
    #[allow(dead_code)]
    pub fn from_directory(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut fp = DirectoryFingerprint::new();

        // Only scan immediate children (not recursive)
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let name = entry.file_name().into_string().unwrap_or_default();

            let mtime = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();

            // Use file size directly (don't recursively calculate dir sizes again)
            let size = metadata.len();

            fp.entries.insert(name, (size, mtime));
        }

        Ok(fp)
    }

    /// Create fingerprint from DirEntry list (very fast - no extra filesystem calls)
    pub fn from_entries(entries: &[crate::scan::DirEntry]) -> Self {
        let mut fp = DirectoryFingerprint::new();

        for entry in entries {
            // Use mtime=0 as a placeholder for now since we have sizes
            // The important part for change detection is the size, not mtime
            fp.entries.insert(entry.name.clone(), (entry.size, 0));
        }

        fp
    }

    /// Compare with another fingerprint and return changes
    pub fn get_changes(&self, other: &DirectoryFingerprint) -> Vec<SizeChange> {
        let mut changes = Vec::new();

        // Check for size changes in existing entries
        for (name, (new_size, _new_mtime)) in &other.entries {
            if let Some((old_size, _old_mtime)) = self.entries.get(name) {
                if old_size != new_size {
                    let delta = *new_size as i64 - *old_size as i64;
                    let percent = if *old_size > 0 {
                        (delta as f32 / *old_size as f32) * 100.0
                    } else {
                        100.0
                    };

                    changes.push(SizeChange {
                        name: name.clone(),
                        old_size: *old_size,
                        new_size: *new_size,
                        delta_bytes: delta,
                        delta_percent: percent,
                    });
                }
            }
        }

        // Sort by absolute delta (largest changes first)
        changes.sort_by(|a, b| b.delta_bytes.abs().cmp(&a.delta_bytes.abs()));

        changes
    }

    /// Save fingerprint to a simple binary format
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut file = fs::File::create(path)?;

        // Format: simple text-based for debuggability
        // name:size:mtime\n
        for (name, (size, mtime)) in &self.entries {
            writeln!(file, "{}:{}:{}", name, size, mtime)?;
        }

        Ok(())
    }

    /// Load fingerprint from saved format
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut fp = DirectoryFingerprint::new();

        if !path.exists() {
            return Ok(fp); // Return empty if no previous snapshot
        }

        let content = fs::read_to_string(path)?;
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                if let (Ok(size), Ok(mtime)) = (parts[1].parse::<u64>(), parts[2].parse::<u64>()) {
                    fp.entries.insert(parts[0].to_string(), (size, mtime));
                }
            }
        }

        Ok(fp)
    }
}

#[allow(dead_code)]
fn calculate_dir_size(path: PathBuf) -> u64 {
    use walkdir::WalkDir;

    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Get the fingerprint file path for a directory
pub fn get_fingerprint_path(dir_path: &Path) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let cache_dir = PathBuf::from(home).join(".mcdu").join("cache");

    // Create a safe filename from the directory path
    let safe_name = dir_path
        .to_string_lossy()
        .replace("/", "_")
        .replace(".", "_");

    cache_dir.join(format!("fp_{}.txt", safe_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_changes_detection() {
        let mut old = DirectoryFingerprint::new();
        old.entries.insert("file1".to_string(), (1000, 100));
        old.entries.insert("file2".to_string(), (2000, 200));

        let mut new = DirectoryFingerprint::new();
        new.entries.insert("file1".to_string(), (2000, 100)); // 1000 bytes larger
        new.entries.insert("file2".to_string(), (2000, 200)); // no change

        let changes = old.get_changes(&new);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "file1");
        assert_eq!(changes[0].delta_bytes, 1000);
    }
}
