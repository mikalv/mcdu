//! Parallel scanning support using rayon
//!
//! Provides configurable multi-threaded scanning with real-time progress updates.

use crate::config::CleanupConfig;
use crate::platform::PlatformPaths;
use crate::rules::{Candidate, MatchType, Rule};
use crate::scanner::{CategoryGroup, ScanProgress};
use glob::Pattern;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::SystemTime;
use walkdir::WalkDir;

/// Context shared between scan operations
struct ScanContext<'a> {
    scan_paths: &'a [PathBuf],
    now: SystemTime,
    found_count: &'a AtomicU64,
    total_size: &'a AtomicU64,
    matched_dirs: &'a Arc<Mutex<HashSet<PathBuf>>>,
    results: &'a mut Vec<Candidate>,
    progress_tx: Option<&'a mpsc::Sender<ScanProgress>>,
}

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Configuration for parallel scanning
#[derive(Debug, Clone)]
pub struct ParallelScanConfig {
    /// Number of threads (0 = auto, based on CPU count)
    pub num_threads: usize,
    /// Ratio of CPUs to use when auto-detecting (0.0-1.0)
    pub cpu_ratio: f64,
}

impl Default for ParallelScanConfig {
    fn default() -> Self {
        Self {
            num_threads: 0,  // Auto
            cpu_ratio: 0.75, // Use 75% of CPUs by default
        }
    }
}

impl ParallelScanConfig {
    /// Get effective number of threads to use
    pub fn effective_threads(&self) -> usize {
        if self.num_threads > 0 {
            self.num_threads
        } else {
            let cpus = num_cpus::get();
            ((cpus as f64 * self.cpu_ratio).ceil() as usize).max(2)
        }
    }
}

/// Get actual disk usage for a file
#[cfg(unix)]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks() * 512
}

#[cfg(not(unix))]
fn disk_usage(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

/// Calculate directory size
fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| disk_usage(&m))
        .sum()
}

/// Parallel scanner with progress reporting
pub struct ParallelScanner {
    config: CleanupConfig,
    platform_paths: PlatformPaths,
    scan_config: ParallelScanConfig,
}

impl ParallelScanner {
    /// Create a new parallel scanner
    pub fn new(
        config: CleanupConfig,
        platform_paths: PlatformPaths,
        scan_config: ParallelScanConfig,
    ) -> Self {
        Self {
            config,
            platform_paths,
            scan_config,
        }
    }

    /// Scan all rules in parallel by category
    pub fn scan(
        &self,
        progress_tx: Option<mpsc::Sender<ScanProgress>>,
        now: SystemTime,
    ) -> Vec<Candidate> {
        let num_threads = self.scan_config.effective_threads();

        // Build custom thread pool
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("mcdu-scan-{}", i))
            .build()
            .expect("Failed to build thread pool");

        // Shared progress counters
        let found_count = Arc::new(AtomicU64::new(0));
        let total_size = Arc::new(AtomicU64::new(0));
        let matched_dirs = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));

        // Resolve scan paths once
        let scan_paths: Vec<PathBuf> = self
            .config
            .scan_paths
            .iter()
            .filter_map(|p| self.platform_paths.resolve_path(p))
            .collect();

        // Get unique categories
        let categories: Vec<String> = self
            .config
            .rules
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.category.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // Process categories in parallel
        let results: Vec<Vec<Candidate>> = pool.install(|| {
            categories
                .par_iter()
                .map(|category| {
                    let category_rules: Vec<&Rule> = self
                        .config
                        .rules
                        .iter()
                        .filter(|r| r.category == *category && r.enabled)
                        .collect();

                    // Send progress
                    if let Some(ref tx) = progress_tx {
                        let _ = tx.send(ScanProgress {
                            current_path: None,
                            found_count: found_count.load(Ordering::Relaxed),
                            total_size: total_size.load(Ordering::Relaxed),
                            current_category: Some(category.clone()),
                        });
                    }

                    let mut category_candidates = Vec::new();
                    let mut ctx = ScanContext {
                        scan_paths: &scan_paths,
                        now,
                        found_count: &found_count,
                        total_size: &total_size,
                        matched_dirs: &matched_dirs,
                        results: &mut category_candidates,
                        progress_tx: progress_tx.as_ref(),
                    };

                    for rule in category_rules {
                        self.scan_rule(rule, &mut ctx);
                    }

                    category_candidates
                })
                .collect()
        });

        // Flatten and sort
        let mut all: Vec<Candidate> = results.into_iter().flatten().collect();
        all.sort_by(|a, b| a.rule_category.cmp(&b.rule_category));
        all
    }

    fn scan_rule(&self, rule: &Rule, ctx: &mut ScanContext) {
        // Handle project_marker rules
        if let Some(ref marker) = rule.project_marker {
            self.scan_project_marker(rule, marker, ctx);
            return;
        }

        // Standard path scanning
        let base_path = match rule.resolve_base_path(&self.platform_paths) {
            Some(p) => p,
            None => return,
        };

        if !base_path.exists() {
            return;
        }

        let walker = {
            let mut w = WalkDir::new(&base_path);
            if let Some(depth) = rule.max_depth {
                w = w.max_depth(depth as usize);
            }
            w
        };

        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            // Check already matched
            {
                let dirs = ctx.matched_dirs.lock().unwrap();
                if dirs.iter().any(|d| path.starts_with(d) && path != d) {
                    continue;
                }
            }

            let is_dir = metadata.is_dir();
            let is_file = metadata.is_file();

            match rule.match_type {
                MatchType::File => {
                    if !is_file {
                        continue;
                    }
                }
                MatchType::Directory => {
                    if !is_dir || path == base_path {
                        continue;
                    }
                }
                MatchType::Both => {
                    if path == base_path {
                        continue;
                    }
                }
            }

            if !rule.matches(&self.platform_paths, path, &metadata, ctx.now) {
                continue;
            }

            let path_buf = path.to_path_buf();
            {
                let dirs = ctx.matched_dirs.lock().unwrap();
                if dirs.contains(&path_buf) {
                    continue;
                }
            }

            let size = if is_dir {
                dir_size(path)
            } else {
                disk_usage(&metadata)
            };
            let is_active = metadata
                .modified()
                .ok()
                .and_then(|m| ctx.now.duration_since(m).ok())
                .map(|d| d < std::time::Duration::from_secs(48 * 3600))
                .unwrap_or(false);

            let candidate = Candidate::new(
                path_buf.clone(),
                rule.name.clone(),
                rule.category.clone(),
                rule.pattern.clone(),
                size,
                metadata.accessed().ok(),
                is_active,
            )
            .with_directory(is_dir)
            .with_warning(rule.warning.clone())
            .with_risky(rule.risky)
            .with_cleanup_command(rule.cleanup_command.clone());

            ctx.results.push(candidate);

            if is_dir {
                ctx.matched_dirs.lock().unwrap().insert(path_buf.clone());
            }

            ctx.found_count.fetch_add(1, Ordering::Relaxed);
            ctx.total_size.fetch_add(size, Ordering::Relaxed);

            if let Some(tx) = ctx.progress_tx {
                let _ = tx.send(ScanProgress {
                    current_path: Some(path_buf),
                    found_count: ctx.found_count.load(Ordering::Relaxed),
                    total_size: ctx.total_size.load(Ordering::Relaxed),
                    current_category: Some(rule.category.clone()),
                });
            }
        }
    }

    fn scan_project_marker(&self, rule: &Rule, marker: &str, ctx: &mut ScanContext) {
        let max_depth = rule.max_depth.unwrap_or(6) as usize;
        let artifact_name = rule
            .pattern
            .trim_start_matches("**/")
            .trim_start_matches("*/");
        let is_glob = marker.contains('*');

        for scan_path in ctx.scan_paths {
            if !scan_path.exists() {
                continue;
            }

            for entry in WalkDir::new(scan_path)
                .max_depth(max_depth)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                // Skip common non-project dirs
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "node_modules"
                        || name == "target"
                        || name == "_build"
                        || name == "vendor"
                    {
                        continue;
                    }
                }

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

                if !has_marker {
                    continue;
                }

                let artifact_path = path.join(artifact_name);

                {
                    let dirs = ctx.matched_dirs.lock().unwrap();
                    if dirs.contains(&artifact_path) {
                        continue;
                    }
                }

                if !artifact_path.exists() {
                    continue;
                }

                let metadata = match std::fs::metadata(&artifact_path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let is_dir = metadata.is_dir();
                match rule.match_type {
                    MatchType::File => {
                        if !metadata.is_file() {
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

                if let Some(min_age) = rule.min_age_hours {
                    if let Ok(modified) = metadata.modified() {
                        let age = std::time::Duration::from_secs(min_age * 3600);
                        if ctx
                            .now
                            .duration_since(modified)
                            .map(|d| d < age)
                            .unwrap_or(true)
                        {
                            continue;
                        }
                    }
                }

                let size = if is_dir {
                    dir_size(&artifact_path)
                } else {
                    disk_usage(&metadata)
                };

                if let Some(min_size) = rule.min_size_bytes {
                    if size < min_size {
                        continue;
                    }
                }

                let is_active = metadata
                    .modified()
                    .ok()
                    .and_then(|m| ctx.now.duration_since(m).ok())
                    .map(|d| d < std::time::Duration::from_secs(48 * 3600))
                    .unwrap_or(false);

                let candidate = Candidate::new(
                    artifact_path.clone(),
                    rule.name.clone(),
                    rule.category.clone(),
                    rule.pattern.clone(),
                    size,
                    metadata.accessed().ok(),
                    is_active,
                )
                .with_directory(is_dir)
                .with_warning(rule.warning.clone())
                .with_risky(rule.risky)
                .with_cleanup_command(rule.cleanup_command.clone());

                ctx.results.push(candidate);
                ctx.matched_dirs
                    .lock()
                    .unwrap()
                    .insert(artifact_path.clone());

                ctx.found_count.fetch_add(1, Ordering::Relaxed);
                ctx.total_size.fetch_add(size, Ordering::Relaxed);

                if let Some(tx) = ctx.progress_tx {
                    let _ = tx.send(ScanProgress {
                        current_path: Some(artifact_path),
                        found_count: ctx.found_count.load(Ordering::Relaxed),
                        total_size: ctx.total_size.load(Ordering::Relaxed),
                        current_category: Some(rule.category.clone()),
                    });
                }
            }
        }
    }

    /// Get number of threads being used
    pub fn thread_count(&self) -> usize {
        self.scan_config.effective_threads()
    }
}

/// Scan with default parallel configuration
pub fn parallel_scan(
    config: &CleanupConfig,
    platform_paths: &PlatformPaths,
    progress_tx: Option<mpsc::Sender<ScanProgress>>,
    now: SystemTime,
) -> Vec<Candidate> {
    let scanner = ParallelScanner::new(
        config.clone(),
        platform_paths.clone(),
        ParallelScanConfig::default(),
    );
    scanner.scan(progress_tx, now)
}

/// Group candidates by category
pub fn group_by_category(candidates: Vec<Candidate>) -> Vec<CategoryGroup> {
    crate::scanner::group_by_category(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CleanupConfig;
    use crate::platform::PlatformPaths;
    use crate::rules::Rule;
    use std::fs;
    use tempfile::tempdir;

    fn platform_paths(tmp: &tempfile::TempDir) -> PlatformPaths {
        PlatformPaths {
            home_dir: tmp.path().join("home"),
            cache_dir: tmp.path().join("cache"),
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
        }
    }

    #[test]
    fn effective_threads_auto() {
        let config = ParallelScanConfig::default();
        let threads = config.effective_threads();
        assert!(threads >= 2);
        assert!(threads <= num_cpus::get());
    }

    #[test]
    fn effective_threads_manual() {
        let config = ParallelScanConfig {
            num_threads: 4,
            cpu_ratio: 0.5,
        };
        assert_eq!(config.effective_threads(), 4);
    }

    #[test]
    fn parallel_scan_finds_files() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);

        // Create test files
        fs::create_dir_all(&paths.cache_dir).unwrap();
        fs::write(paths.cache_dir.join("test.log"), "test").unwrap();

        let config = CleanupConfig {
            scan_paths: vec!["${CACHE_DIR}".into()],
            rules: vec![Rule {
                name: "logs".into(),
                category: "Test".into(),
                pattern: "**/*.log".into(),
                path: "${CACHE_DIR}".into(),
                ..Default::default()
            }],
        };

        let scanner = ParallelScanner::new(config, paths, ParallelScanConfig::default());
        let results = scanner.scan(None, SystemTime::now());

        assert_eq!(results.len(), 1);
        assert!(results[0].path.ends_with("test.log"));
    }
}
