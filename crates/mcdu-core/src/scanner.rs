use crate::config::CleanupConfig;
use crate::platform::PlatformPaths;
use crate::rules::{Candidate, MatchType, Rule};
use glob::Pattern;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;
use walkdir::WalkDir;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScanProgress {
    pub current_path: Option<PathBuf>,
    pub found_count: u64,
    pub total_size: u64,
    pub current_category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CategoryGroup {
    pub name: String,
    pub candidates: Vec<Candidate>,
}

pub fn scan(
    config: &CleanupConfig,
    platform_paths: &PlatformPaths,
    progress_tx: Option<mpsc::Sender<ScanProgress>>,
    now: SystemTime,
) -> Vec<Candidate> {
    let mut results = Vec::new();
    let mut found_count = 0u64;
    let mut total_size = 0u64;
    let scan_paths: Vec<PathBuf> = config
        .scan_paths
        .iter()
        .filter_map(|p| platform_paths.resolve_path(p))
        .collect();

    // Track matched paths (files and dirs) to avoid duplicates across rules
    let mut matched_paths: HashSet<PathBuf> = HashSet::new();

    for rule in &config.rules {
        scan_rule(
            rule,
            platform_paths,
            progress_tx.as_ref(),
            now,
            &mut found_count,
            &mut total_size,
            &mut results,
            &scan_paths,
            &mut matched_paths,
        );
    }

    results
}

/// Calculate directory size recursively
fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .same_file_system(true)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| disk_usage(&m))
        .sum()
}

/// Get actual disk usage for a file (handles sparse files correctly on Unix)
#[cfg(unix)]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

/// Skip directories that are build artifacts or hidden (never contain project roots)
fn should_skip_dir(name: &str) -> bool {
    (name.starts_with('.') && name != ".git")
        || matches!(
            name,
            "node_modules" | "target" | "_build" | "vendor" | "build" | "dist" | "__pycache__"
        )
}

/// Find project roots containing a specific marker file
fn find_project_roots(scan_paths: &[PathBuf], marker: &str, max_depth: usize) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let is_glob = marker.contains('*');

    for scan_path in scan_paths {
        if !scan_path.exists() {
            continue;
        }

        // filter_entry prunes entire subtrees (vs continue which only skips one entry)
        let walker = WalkDir::new(scan_path)
            .max_depth(max_depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                e.file_type().is_dir()
                    && e.file_name()
                        .to_str()
                        .map(|n| !should_skip_dir(n))
                        .unwrap_or(true)
            });

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();

            let has_marker = if is_glob {
                if let Ok(pattern) = Pattern::new(marker) {
                    std::fs::read_dir(path)
                        .map(|entries| {
                            entries.filter_map(|e| e.ok()).any(|e| {
                                e.file_name()
                                    .to_str()
                                    .map(|n| pattern.matches(n))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                path.join(marker).exists()
            };

            if has_marker {
                roots.push(path.to_path_buf());
            }
        }
    }

    roots
}

#[allow(clippy::too_many_arguments)]
fn scan_rule(
    rule: &Rule,
    platform_paths: &PlatformPaths,
    progress_tx: Option<&mpsc::Sender<ScanProgress>>,
    now: SystemTime,
    found_count: &mut u64,
    total_size: &mut u64,
    results: &mut Vec<Candidate>,
    scan_paths: &[PathBuf],
    matched_paths: &mut HashSet<PathBuf>,
) {
    // Send progress update for category
    if let Some(tx) = progress_tx {
        let _ = tx.send(ScanProgress {
            current_path: None,
            found_count: *found_count,
            total_size: *total_size,
            current_category: Some(rule.category.clone()),
        });
    }

    // Handle project_marker rules differently
    if let Some(ref marker) = rule.project_marker {
        scan_project_marker_rule(
            rule,
            marker,
            platform_paths,
            progress_tx,
            now,
            found_count,
            total_size,
            results,
            scan_paths,
            matched_paths,
        );
        return;
    }

    // Standard path-based scanning
    let base_path = match rule.resolve_base_path(platform_paths) {
        Some(path) => path,
        None => return,
    };

    // Restrict to intersection of base_path and requested scan_paths
    let walk_roots = resolve_walk_roots(&base_path, scan_paths);
    if walk_roots.is_empty() {
        return;
    }

    for walk_root in walk_roots {
        if !walk_root.exists() {
            continue;
        }
        scan_path_with_rule(
            rule,
            &walk_root,
            platform_paths,
            progress_tx,
            now,
            found_count,
            total_size,
            results,
            matched_paths,
        );
    }
}

/// Compute walk roots as the intersection of a rule base_path and scan_paths.
fn resolve_walk_roots(base_path: &Path, scan_paths: &[PathBuf]) -> Vec<PathBuf> {
    if scan_paths.is_empty() {
        return vec![base_path.to_path_buf()];
    }
    let mut roots = Vec::new();
    for sp in scan_paths {
        if sp.starts_with(base_path) {
            // scan path is inside rule base → walk only the scan path
            roots.push(sp.clone());
        } else if base_path.starts_with(sp) {
            // rule base is inside scan path → walk the (narrower) rule base
            roots.push(base_path.to_path_buf());
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

/// Scan for artifacts in project directories identified by marker files
#[allow(clippy::too_many_arguments)]
fn scan_project_marker_rule(
    rule: &Rule,
    marker: &str,
    _platform_paths: &PlatformPaths,
    progress_tx: Option<&mpsc::Sender<ScanProgress>>,
    now: SystemTime,
    found_count: &mut u64,
    total_size: &mut u64,
    results: &mut Vec<Candidate>,
    scan_paths: &[PathBuf],
    matched_paths: &mut HashSet<PathBuf>,
) {
    let max_depth = rule.max_depth.unwrap_or(6) as usize;
    let project_roots = find_project_roots(scan_paths, marker, max_depth);

    // Extract the artifact name from the pattern (e.g., "**/target" -> "target")
    let artifact_name = rule
        .pattern
        .trim_start_matches("**/")
        .trim_start_matches("*/")
        .to_string();

    for project_root in project_roots {
        let artifact_path = project_root.join(&artifact_name);

        // Skip if already matched
        if matched_paths.contains(&artifact_path) {
            continue;
        }

        if !artifact_path.exists() {
            continue;
        }

        let metadata = match std::fs::metadata(&artifact_path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Check match type
        let is_dir = metadata.is_dir();
        let is_file = metadata.is_file();
        match rule.match_type {
            MatchType::File => {
                if !is_file {
                    continue;
                }
            }
            MatchType::Directory => {
                if !is_dir {
                    continue;
                }
            }
            MatchType::Both => {}
        }

        // Check age if specified
        if let Some(min_age_hours) = rule.min_age_hours {
            if let Ok(modified) = metadata.modified() {
                let min_age = std::time::Duration::from_secs(min_age_hours * 3600);
                if let Ok(age) = now.duration_since(modified) {
                    if age < min_age {
                        continue;
                    }
                }
            }
        }

        // Calculate size
        let size = if is_dir {
            dir_size(&artifact_path)
        } else {
            disk_usage(&metadata)
        };

        // Check min size if specified
        if let Some(min_size) = rule.min_size_bytes {
            if size < min_size {
                continue;
            }
        }

        let last_accessed = metadata.accessed().ok();
        let is_active = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|duration| duration < std::time::Duration::from_secs(48 * 3600))
            .unwrap_or(false);

        let candidate = Candidate::new(
            artifact_path.clone(),
            rule.name.clone(),
            rule.category.clone(),
            rule.pattern.clone(),
            size,
            last_accessed,
            is_active,
        )
        .with_directory(is_dir)
        .with_warning(rule.warning.clone())
        .with_risky(rule.risky)
        .with_cleanup_command(rule.cleanup_command.clone());

        results.push(candidate);
        matched_paths.insert(artifact_path.clone());

        *found_count += 1;
        *total_size += size;

        if let Some(tx) = progress_tx {
            let _ = tx.send(ScanProgress {
                current_path: Some(artifact_path),
                found_count: *found_count,
                total_size: *total_size,
                current_category: Some(rule.category.clone()),
            });
        }
    }
}

/// Scan a specific path with a rule
#[allow(clippy::too_many_arguments)]
fn scan_path_with_rule(
    rule: &Rule,
    base_path: &Path,
    platform_paths: &PlatformPaths,
    progress_tx: Option<&mpsc::Sender<ScanProgress>>,
    now: SystemTime,
    found_count: &mut u64,
    total_size: &mut u64,
    results: &mut Vec<Candidate>,
    matched_paths: &mut HashSet<PathBuf>,
) {
    // Build walker with max_depth if specified
    let walker = {
        let mut w = WalkDir::new(base_path).follow_links(false);
        if let Some(depth) = rule.max_depth {
            w = w.max_depth(depth as usize);
        }
        w
    };

    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();

        // Skip if inside an already-matched directory
        if matched_paths
            .iter()
            .any(|d| path.starts_with(d) && path != d)
        {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let is_dir = metadata.is_dir();
        let is_file = metadata.is_file();

        // Skip based on match_type
        match rule.match_type {
            MatchType::File => {
                if !is_file {
                    continue;
                }
            }
            MatchType::Directory => {
                if !is_dir {
                    continue;
                }
                // For directories, skip the base path itself
                if path == base_path {
                    continue;
                }
            }
            MatchType::Both => {
                // Skip base path for both types
                if path == base_path {
                    continue;
                }
            }
        }

        if !rule.matches(platform_paths, path, &metadata, now) {
            continue;
        }

        // Skip if already matched
        let path_buf = path.to_path_buf();
        if matched_paths.contains(&path_buf) {
            continue;
        }

        // Calculate size - for directories, sum contents
        let size = if is_dir {
            dir_size(path)
        } else {
            disk_usage(&metadata)
        };

        let last_accessed = metadata.accessed().ok();
        let is_active = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|duration| duration < std::time::Duration::from_secs(48 * 3600))
            .unwrap_or(false);

        let candidate = Candidate::new(
            path_buf.clone(),
            rule.name.clone(),
            rule.category.clone(),
            rule.pattern.clone(),
            size,
            last_accessed,
            is_active,
        )
        .with_directory(is_dir)
        .with_warning(rule.warning.clone())
        .with_risky(rule.risky)
        .with_cleanup_command(rule.cleanup_command.clone());

        results.push(candidate);

        // Track all matched paths (files and dirs) to avoid duplicates across rules
        matched_paths.insert(path_buf.clone());

        *found_count += 1;
        *total_size += size;

        if let Some(tx) = progress_tx {
            let _ = tx.send(ScanProgress {
                current_path: Some(path_buf),
                found_count: *found_count,
                total_size: *total_size,
                current_category: Some(rule.category.clone()),
            });
        }
    }
}

pub fn group_by_category(candidates: Vec<Candidate>) -> Vec<CategoryGroup> {
    let mut grouped: std::collections::BTreeMap<String, Vec<Candidate>> =
        std::collections::BTreeMap::new();
    for cand in candidates {
        grouped
            .entry(cand.rule_category.clone())
            .or_default()
            .push(cand);
    }

    grouped
        .into_iter()
        .map(|(name, candidates)| CategoryGroup { name, candidates })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CleanupConfig;
    use crate::platform::PlatformPaths;
    use crate::rules::Rule;
    use std::fs;
    use std::io::Write;
    use std::time::Duration;
    use tempfile::tempdir;

    fn platform_paths(tmp: &tempfile::TempDir) -> PlatformPaths {
        PlatformPaths {
            home_dir: tmp.path().join("home"),
            cache_dir: tmp.path().join("cache"),
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
        }
    }

    fn rule_for_cache() -> Rule {
        Rule {
            name: "logs".into(),
            category: "cache".into(),
            pattern: "**/*.log".into(),
            path: "${CACHE_DIR}".into(),
            min_age_hours: Some(1),
            ..Default::default()
        }
    }

    #[test]
    fn scan_collects_matching_candidates() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let rule = rule_for_cache();
        let config = CleanupConfig {
            scan_paths: vec!["${CACHE_DIR}".into()],
            rules: vec![rule.clone()],
        };

        let target_dir = paths.cache_dir.join("nested");
        fs::create_dir_all(&target_dir).unwrap();
        let file_path = target_dir.join("file.log");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "hello").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let now = metadata
            .modified()
            .unwrap()
            .checked_add(Duration::from_secs(7200))
            .unwrap();

        let results = scan(&config, &paths, None, now);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, file_path);
        assert_eq!(results[0].rule_name, rule.name);
        assert_eq!(results[0].rule_category, rule.category);
        assert_eq!(results[0].rule_pattern, rule.pattern);
        assert!(!results[0].is_directory);
    }

    #[test]
    fn scan_finds_directories() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let rule = Rule {
            name: "target-dirs".into(),
            category: "rust".into(),
            pattern: "**/target".into(),
            path: "${CACHE_DIR}".into(),
            match_type: MatchType::Directory,
            ..Default::default()
        };

        let config = CleanupConfig {
            scan_paths: vec!["${CACHE_DIR}".into()],
            rules: vec![rule],
        };

        // Create a target directory with some files
        let target_dir = paths.cache_dir.join("project").join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("build.o"), "object file").unwrap();

        let now = SystemTime::now();
        let results = scan(&config, &paths, None, now);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, target_dir);
        assert!(results[0].is_directory);
        assert!(results[0].size_bytes > 0);
    }

    #[test]
    fn scan_with_project_marker() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);

        // Create a Rust project with target directory
        let project_dir = paths.home_dir.join("myproject");
        let target_dir = project_dir.join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(project_dir.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        fs::write(target_dir.join("debug.o"), "build artifact").unwrap();

        let rule = Rule {
            name: "rust-target".into(),
            category: "Rust/Cargo".into(),
            pattern: "**/target".into(),
            path: "${HOME}".into(),
            match_type: MatchType::Directory,
            project_marker: Some("Cargo.toml".into()),
            max_depth: Some(5),
            ..Default::default()
        };

        let config = CleanupConfig {
            scan_paths: vec!["${HOME}".into()],
            rules: vec![rule],
        };

        let now = SystemTime::now();
        let results = scan(&config, &paths, None, now);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, target_dir);
        assert!(results[0].is_directory);
    }

    #[test]
    fn scan_reports_progress() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let rule = rule_for_cache();
        let config = CleanupConfig {
            scan_paths: vec!["${CACHE_DIR}".into()],
            rules: vec![rule],
        };

        let file_path = paths.cache_dir.join("file.log");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, "hello").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let now = metadata
            .modified()
            .unwrap()
            .checked_add(Duration::from_secs(7200))
            .unwrap();

        let (tx, rx) = mpsc::channel();
        let results = scan(&config, &paths, Some(tx), now);
        assert!(!results.is_empty());
        let progress: Vec<_> = rx.try_iter().collect();
        assert!(!progress.is_empty());
    }

    #[test]
    fn groups_candidates_by_category() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let rule_a = Rule {
            name: "logs".into(),
            category: "cat-a".into(),
            pattern: "**/*.log".into(),
            path: "${CACHE_DIR}".into(),
            ..Default::default()
        };
        let rule_b = Rule {
            name: "txt".into(),
            category: "cat-b".into(),
            pattern: "**/*.txt".into(),
            path: "${CACHE_DIR}".into(),
            ..Default::default()
        };

        let config = CleanupConfig {
            scan_paths: vec!["${CACHE_DIR}".into()],
            rules: vec![rule_a, rule_b],
        };

        let file_a = paths.cache_dir.join("one.log");
        let file_b = paths.cache_dir.join("two.txt");
        fs::create_dir_all(paths.cache_dir.clone()).unwrap();
        fs::write(&file_a, "a").unwrap();
        fs::write(&file_b, "b").unwrap();
        let now = SystemTime::now();

        let grouped = group_by_category(scan(&config, &paths, None, now));
        let mut cats: Vec<_> = grouped.iter().map(|g| g.name.as_str()).collect();
        cats.sort();
        assert_eq!(cats, vec!["cat-a", "cat-b"]);
    }

    #[test]
    fn avoids_duplicate_matches() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);

        // Two rules that could match the same directory
        let rule1 = Rule {
            name: "target-1".into(),
            category: "rust".into(),
            pattern: "**/target".into(),
            path: "${CACHE_DIR}".into(),
            match_type: MatchType::Directory,
            ..Default::default()
        };
        let rule2 = Rule {
            name: "target-2".into(),
            category: "rust".into(),
            pattern: "**/target".into(),
            path: "${CACHE_DIR}".into(),
            match_type: MatchType::Directory,
            ..Default::default()
        };

        let config = CleanupConfig {
            scan_paths: vec!["${CACHE_DIR}".into()],
            rules: vec![rule1, rule2],
        };

        let target_dir = paths.cache_dir.join("project").join("target");
        fs::create_dir_all(&target_dir).unwrap();

        let now = SystemTime::now();
        let results = scan(&config, &paths, None, now);

        // Should only match once, not twice
        assert_eq!(results.len(), 1);
    }
}
