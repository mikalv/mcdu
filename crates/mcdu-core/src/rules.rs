use crate::platform::PlatformPaths;
use glob::Pattern;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::SystemTime;

/// Match type for rules - whether to match files, directories, or both
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    #[default]
    File,
    Directory,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub category: String,
    pub pattern: String,
    pub path: String,

    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub min_age_hours: Option<u64>,
    #[serde(default)]
    pub min_size_bytes: Option<u64>,
    #[serde(default)]
    pub risky: bool,
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    // NEW: Match type - file, directory, or both
    #[serde(default)]
    pub match_type: MatchType,

    // NEW: Warning message for risky rules
    #[serde(default)]
    pub warning: Option<String>,

    // NEW: Project marker - scan for this pattern in project directories
    #[serde(default)]
    pub project_marker: Option<String>,

    // NEW: Dynamic path detection via shell command
    #[serde(default)]
    pub command: Option<String>,

    // NEW: Cleanup command (alternative to deletion)
    #[serde(default)]
    pub cleanup_command: Option<String>,

    // NEW: Maximum scan depth (None = unlimited)
    #[serde(default)]
    pub max_depth: Option<u32>,

    // NEW: Exclude patterns (glob patterns to skip)
    #[serde(default)]
    pub exclude: Vec<String>,

    // NEW: Description for UI display
    #[serde(default)]
    pub description: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            name: String::new(),
            category: String::new(),
            pattern: String::new(),
            path: String::new(),
            signature: None,
            min_age_hours: None,
            min_size_bytes: None,
            risky: false,
            enabled: true,
            match_type: MatchType::default(),
            warning: None,
            project_marker: None,
            command: None,
            cleanup_command: None,
            max_depth: None,
            exclude: vec![],
            description: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub rule_name: String,
    pub rule_category: String,
    pub rule_pattern: String,
    pub size_bytes: u64,
    pub last_accessed: Option<SystemTime>,
    pub is_active: bool,
    pub is_directory: bool,
    pub warning: Option<String>,
    pub default_selected: bool,
    pub risky: bool,
    /// If set, cleanup runs this shell command instead of deleting/quarantining.
    /// Templates: `{path}` (absolute match path), `{dir}` (directory to run in).
    pub cleanup_command: Option<String>,
}

impl Rule {
    pub fn glob(&self) -> Result<Pattern, glob::PatternError> {
        Pattern::new(&self.pattern)
    }

    pub fn base_path(&self, platform_paths: &PlatformPaths) -> Option<PathBuf> {
        platform_paths.resolve_path(&self.path)
    }

    /// Get base path, potentially from a shell command
    pub fn resolve_base_path(&self, platform_paths: &PlatformPaths) -> Option<PathBuf> {
        // First try command if present
        if let Some(cmd) = &self.command {
            if let Ok(output) = std::process::Command::new("sh").arg("-c").arg(cmd).output() {
                if output.status.success() {
                    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path_str.is_empty() {
                        let path = PathBuf::from(&path_str);
                        if path.exists() {
                            return Some(path);
                        }
                    }
                }
            }
        }

        // Fall back to template path
        self.base_path(platform_paths)
    }

    /// Check if a path matches any exclude pattern
    fn is_excluded(&self, path: &Path) -> bool {
        for exclude_pattern in &self.exclude {
            if let Ok(pattern) = Pattern::new(exclude_pattern) {
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    if pattern.matches(filename) {
                        return true;
                    }
                }
                // Also check full path
                if let Some(path_str) = path.to_str() {
                    if pattern.matches(path_str) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn matches(
        &self,
        platform_paths: &PlatformPaths,
        candidate_path: &Path,
        metadata: &fs::Metadata,
        now: SystemTime,
    ) -> bool {
        if !self.enabled {
            return false;
        }

        // Check match type
        let is_dir = metadata.is_dir();
        let is_file = metadata.is_file();
        match self.match_type {
            MatchType::File => {
                if !is_file {
                    return false;
                }
            }
            MatchType::Directory => {
                if !is_dir {
                    return false;
                }
            }
            MatchType::Both => {} // Accept either
        }

        let base_path = match self.resolve_base_path(platform_paths) {
            Some(path) => path,
            None => return false,
        };

        if let Some(signature) = &self.signature {
            if !base_path.join(signature).exists() {
                return false;
            }
        }

        let pattern = match self.glob() {
            Ok(p) => p,
            Err(_) => return false,
        };

        let relative = candidate_path
            .strip_prefix(&base_path)
            .unwrap_or(candidate_path);
        if !pattern.matches_path(relative) {
            return false;
        }

        // Check excludes
        if self.is_excluded(candidate_path) {
            return false;
        }

        // min_size_bytes for files only here; directories are checked after recursive sizing
        if let Some(min_size) = self.min_size_bytes {
            if metadata.is_file() && metadata.len() < min_size {
                return false;
            }
        }

        if let Some(min_age_hours) = self.min_age_hours {
            if let Ok(modified) = metadata.modified() {
                let min_age = Duration::from_secs(min_age_hours * 3600);
                if let Ok(age) = now.duration_since(modified) {
                    if age < min_age {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }

        true
    }
}

impl Candidate {
    pub fn new(
        path: PathBuf,
        rule_name: String,
        rule_category: String,
        rule_pattern: String,
        size_bytes: u64,
        last_accessed: Option<SystemTime>,
        is_active: bool,
    ) -> Self {
        Candidate {
            path,
            rule_name,
            rule_category,
            rule_pattern,
            size_bytes,
            last_accessed,
            is_active,
            is_directory: false,
            warning: None,
            default_selected: true,
            risky: false,
            cleanup_command: None,
        }
    }

    pub fn with_directory(mut self, is_dir: bool) -> Self {
        self.is_directory = is_dir;
        self
    }

    pub fn with_warning(mut self, warning: Option<String>) -> Self {
        self.warning = warning;
        self
    }

    pub fn with_default_selected(mut self, selected: bool) -> Self {
        self.default_selected = selected;
        self
    }

    /// Mark candidate as risky; risky items are not selected by default.
    pub fn with_risky(mut self, risky: bool) -> Self {
        self.risky = risky;
        if risky {
            self.default_selected = false;
        }
        self
    }

    pub fn with_cleanup_command(mut self, cmd: Option<String>) -> Self {
        self.cleanup_command = cmd;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;
    use tempfile::tempdir;

    fn build_rule() -> Rule {
        Rule {
            name: "old-logs".to_string(),
            category: "logs".to_string(),
            pattern: "**/*.log".to_string(),
            path: "${CACHE_DIR}".to_string(),
            min_age_hours: Some(1),
            min_size_bytes: Some(1),
            ..Default::default()
        }
    }

    fn platform_paths(tmp: &tempfile::TempDir) -> PlatformPaths {
        PlatformPaths {
            home_dir: tmp.path().join("home"),
            cache_dir: tmp.path().join("cache"),
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
        }
    }

    #[test]
    fn matches_glob_and_age_and_size() {
        let tmp = tempdir().unwrap();
        let paths = platform_paths(&tmp);
        let rule = build_rule();

        let target_dir = paths.cache_dir.join("nested");
        std::fs::create_dir_all(&target_dir).unwrap();
        let target_file = target_dir.join("file.log");
        let mut file = std::fs::File::create(&target_file).unwrap();
        writeln!(file, "hello").unwrap();

        let metadata = std::fs::metadata(&target_file).unwrap();
        let now = metadata
            .modified()
            .unwrap()
            .checked_add(Duration::from_secs(7200))
            .unwrap();

        assert!(rule.matches(&paths, &target_file, &metadata, now));
    }

    #[test]
    fn fails_when_signature_missing() {
        let tmp = tempdir().unwrap();
        let mut rule = build_rule();
        rule.signature = Some("marker.txt".to_string());

        let paths = platform_paths(&tmp);
        let target_file = paths.cache_dir.join("file.log");
        std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
        std::fs::write(&target_file, "hi").unwrap();
        let metadata = std::fs::metadata(&target_file).unwrap();
        let now = metadata.modified().unwrap();

        assert!(!rule.matches(&paths, &target_file, &metadata, now));
    }

    #[test]
    fn disabled_rule_never_matches() {
        let tmp = tempdir().unwrap();
        let mut rule = build_rule();
        rule.enabled = false;

        let paths = platform_paths(&tmp);
        let target_file = paths.cache_dir.join("file.log");
        std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
        std::fs::write(&target_file, "hi").unwrap();
        let metadata = std::fs::metadata(&target_file).unwrap();
        let now = metadata.modified().unwrap();

        assert!(!rule.matches(&paths, &target_file, &metadata, now));
    }

    #[test]
    fn matches_directory_type() {
        let tmp = tempdir().unwrap();
        let rule = Rule {
            name: "target-dirs".into(),
            category: "rust".into(),
            pattern: "**/target".into(),
            path: "${CACHE_DIR}".into(),
            match_type: MatchType::Directory,
            ..Default::default()
        };

        let paths = platform_paths(&tmp);
        let target_dir = paths.cache_dir.join("project").join("target");
        std::fs::create_dir_all(&target_dir).unwrap();

        let metadata = std::fs::metadata(&target_dir).unwrap();
        let now = SystemTime::now();

        assert!(rule.matches(&paths, &target_dir, &metadata, now));
    }

    #[test]
    fn excludes_patterns() {
        let tmp = tempdir().unwrap();
        let mut rule = build_rule();
        rule.exclude = vec!["*.keep".to_string()];
        rule.pattern = "**/*".into();
        rule.min_age_hours = None;
        rule.min_size_bytes = None;

        let paths = platform_paths(&tmp);
        let target_file = paths.cache_dir.join("file.keep");
        std::fs::create_dir_all(target_file.parent().unwrap()).unwrap();
        std::fs::write(&target_file, "hi").unwrap();
        let metadata = std::fs::metadata(&target_file).unwrap();
        let now = SystemTime::now();

        assert!(!rule.matches(&paths, &target_file, &metadata, now));
    }
}
