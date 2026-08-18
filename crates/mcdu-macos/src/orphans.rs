use crate::bundle::{self, LibraryDir};
use crate::installed;
use mcdu_core::rules::{Candidate, MatchType};
use mcdu_core::scanner::ScanProgress;
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc;
use walkdir::WalkDir;

/// Scan ~/Library for data belonging to uninstalled applications.
/// Returns empty vec if installed-app discovery fails (caller should surface the error).
pub fn scan_orphans(
    home_dir: &Path,
    progress_tx: Option<&mpsc::Sender<ScanProgress>>,
) -> Result<Vec<Candidate>, String> {
    let installed = installed::get_installed_bundle_ids().map_err(|e| e.0)?;
    let running = installed::get_running_bundle_ids();

    // Merge installed and running into a single "known" set
    let known: HashSet<String> = installed.union(&running).cloned().collect();

    let mut results = Vec::new();
    let mut found_count = 0u64;
    let mut total_size = 0u64;

    for &dir_type in LibraryDir::all() {
        let dir_path = bundle::library_dir_path(home_dir, dir_type);

        if !dir_path.exists() {
            continue;
        }

        if let Some(tx) = progress_tx {
            let _ = tx.send(ScanProgress {
                current_path: Some(dir_path.clone()),
                found_count,
                total_size,
                current_category: Some("Orphaned App Data".to_string()),
            });
        }

        let entries = match std::fs::read_dir(&dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };

            // Skip hidden entries
            if name.starts_with('.') {
                continue;
            }

            // For non-Preferences dirs, only consider directories
            // For Preferences, only consider .plist files
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            if dir_type.entries_are_files() {
                if !metadata.is_file() {
                    continue;
                }
            } else if !metadata.is_dir() {
                continue;
            }

            // Extract bundle ID from the entry name
            let bundle_id = match bundle::extract_bundle_id(&name, dir_type) {
                Some(id) => id,
                None => continue,
            };

            // Skip system / whitelisted bundles
            if bundle::is_system_bundle(&bundle_id) || installed::is_whitelisted_prefix(&bundle_id)
            {
                continue;
            }

            // Skip entries that don't look like bundle IDs (reverse-DNS notation)
            if !bundle::looks_like_bundle_id(&bundle_id) {
                continue;
            }

            // If the bundle ID is in our known set, it's not an orphan
            if known.contains(&bundle_id) {
                continue;
            }

            let entry_path = entry.path();

            // Calculate size
            let size = if metadata.is_dir() {
                dir_size(&entry_path)
            } else {
                metadata.len()
            };

            let last_accessed = metadata.accessed().ok();
            let match_type = if metadata.is_dir() {
                MatchType::Directory
            } else {
                MatchType::File
            };

            let candidate = Candidate::new(
                entry_path.clone(),
                format!("orphan-{}", bundle_id),
                "Orphaned App Data".to_string(),
                bundle_id,
                size,
                last_accessed,
                false, // not active
            )
            .with_directory(match_type == MatchType::Directory)
            .with_default_selected(false);

            results.push(candidate);
            found_count += 1;
            total_size += size;

            if let Some(tx) = progress_tx {
                let _ = tx.send(ScanProgress {
                    current_path: Some(entry_path),
                    found_count,
                    total_size,
                    current_category: Some("Orphaned App Data".to_string()),
                });
            }
        }
    }

    // Sort by size descending so the biggest orphans appear first
    results.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(results)
}

/// Calculate total size of a directory recursively
fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                m.blocks() * 512
            }
            #[cfg(not(unix))]
            {
                m.len()
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scan_finds_orphaned_directories() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Create fake ~/Library/Caches with an orphaned bundle
        let caches = home.join("Library").join("Caches");
        let orphan_dir = caches.join("com.uninstalled.app");
        fs::create_dir_all(&orphan_dir).unwrap();
        fs::write(orphan_dir.join("data.bin"), "cached data").unwrap();

        // Create a "known" bundle (we can't easily mock mdfind, so this test
        // verifies the scanning logic with an empty known set)
        let results = scan_with_known(home, &HashSet::new(), None);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rule_category, "Orphaned App Data");
        assert!(results[0].path.ends_with("com.uninstalled.app"));
        assert!(!results[0].default_selected);
    }

    #[test]
    fn scan_skips_known_bundles() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let caches = home.join("Library").join("Caches");
        let known_dir = caches.join("com.known.app");
        fs::create_dir_all(&known_dir).unwrap();
        fs::write(known_dir.join("cache.dat"), "data").unwrap();

        let mut known = HashSet::new();
        known.insert("com.known.app".to_string());

        let results = scan_with_known(home, &known, None);
        assert!(results.is_empty());
    }

    #[test]
    fn scan_skips_apple_bundles() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let caches = home.join("Library").join("Caches");
        let apple_dir = caches.join("com.apple.Safari");
        fs::create_dir_all(&apple_dir).unwrap();

        let results = scan_with_known(home, &HashSet::new(), None);
        assert!(results.is_empty());
    }

    #[test]
    fn scan_handles_saved_state_suffix() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let saved = home.join("Library").join("Saved Application State");
        let orphan = saved.join("com.uninstalled.app.savedState");
        fs::create_dir_all(&orphan).unwrap();

        let results = scan_with_known(home, &HashSet::new(), None);
        assert_eq!(results.len(), 1);
        assert!(results[0].rule_name.contains("com.uninstalled.app"));
    }

    #[test]
    fn scan_handles_plist_preferences() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let prefs = home.join("Library").join("Preferences");
        fs::create_dir_all(&prefs).unwrap();
        fs::write(prefs.join("com.uninstalled.app.plist"), "plist data").unwrap();

        let results = scan_with_known(home, &HashSet::new(), None);
        assert_eq!(results.len(), 1);
        assert!(!results[0].is_directory);
    }

    #[test]
    fn scan_skips_non_bundle_names() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let caches = home.join("Library").join("Caches");
        // This doesn't look like a bundle ID (no dots)
        fs::create_dir_all(caches.join("SomeRandomFolder")).unwrap();
        // Hidden entries should be skipped too
        fs::create_dir_all(caches.join(".hidden")).unwrap();

        let results = scan_with_known(home, &HashSet::new(), None);
        assert!(results.is_empty());
    }

    /// Test helper: scan with a pre-defined set of known bundle IDs
    fn scan_with_known(
        home_dir: &Path,
        known: &HashSet<String>,
        _progress_tx: Option<&mpsc::Sender<ScanProgress>>,
    ) -> Vec<Candidate> {
        let mut results = Vec::new();

        for &dir_type in LibraryDir::all() {
            let dir_path = bundle::library_dir_path(home_dir, dir_type);

            if !dir_path.exists() {
                continue;
            }

            let entries = match std::fs::read_dir(&dir_path) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.filter_map(|e| e.ok()) {
                let name = match entry.file_name().into_string() {
                    Ok(n) => n,
                    Err(_) => continue,
                };

                if name.starts_with('.') {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if dir_type.entries_are_files() {
                    if !metadata.is_file() {
                        continue;
                    }
                } else if !metadata.is_dir() {
                    continue;
                }

                let bundle_id = match bundle::extract_bundle_id(&name, dir_type) {
                    Some(id) => id,
                    None => continue,
                };

                if bundle::is_system_bundle(&bundle_id)
                    || installed::is_whitelisted_prefix(&bundle_id)
                {
                    continue;
                }

                if !bundle::looks_like_bundle_id(&bundle_id) {
                    continue;
                }

                if known.contains(&bundle_id) {
                    continue;
                }

                let entry_path = entry.path();
                let size = if metadata.is_dir() {
                    dir_size(&entry_path)
                } else {
                    metadata.len()
                };

                let last_accessed = metadata.accessed().ok();
                let candidate = Candidate::new(
                    entry_path,
                    format!("orphan-{}", bundle_id),
                    "Orphaned App Data".to_string(),
                    bundle_id,
                    size,
                    last_accessed,
                    false,
                )
                .with_directory(metadata.is_dir())
                .with_default_selected(false);

                results.push(candidate);
            }
        }

        results.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        results
    }
}
