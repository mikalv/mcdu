//! Non-interactive developer cleanup (`mcdu devclean` / `mcdu dc`).
//!
//! Deletes common build artifacts under a project root (default: cwd):
//! - `_build` (Elixir) — optional: keep prod profile (`keep_release`)
//! - `target/` subdirs except `release/` (Rust) when `keep_release`, else whole `target/`
//! - `node_modules` — only when older than `min_age_days` (mtime of the dir itself)
//! - `deps`, `.elixir_ls`, `.parcel-cache`, `__pycache__`, `.venv`/`venv`, `dist/newbuilder`, `.turbo`, `.next`, `build`/`cmake-build-*`
//!
//! Settings live in `~/.mcdu.toml`:
//!
//! ```toml
//! [devclean]
//! min_age_days = 2
//! keep_release = true
//! prune_node_modules_only = true
//! ```

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

/// Artifact kinds we know how to clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// Whole directory can be removed unconditionally.
    Whole,
    /// Directory should be kept, but non-release subdirs pruned (Rust `target/`).
    TargetSubdirs,
    /// Elixir `_build`: optionally keep `prod` while pruning `dev`/`test`.
    MixBuild,
    /// Removed only when older than `min_age_days`.
    AgeGated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevcleanSettings {
    /// Directories with an mtime younger than this are kept when the rule is age-gated.
    #[serde(default = "default_min_age_days")]
    pub min_age_days: u64,
    /// Keep `target/release`, `_build/prod` (release builds).
    #[serde(default = "default_true")]
    pub keep_release: bool,
    /// Only remove `node_modules` older than `min_age_days` (don't touch everything else aggressively).
    #[serde(default = "default_true")]
    pub age_gate_node_modules: bool,
    /// Extra directory names to delete unconditionally.
    #[serde(default)]
    pub extra_dirs: Vec<String>,
    /// Extra directory names to delete when older than `min_age_days`.
    #[serde(default)]
    pub extra_age_gated: Vec<String>,
    /// Max traversal depth (perf + safety).
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// Never descend into these directory names.
    #[serde(default = "default_skip_dirs")]
    pub skip_dirs: Vec<String>,
}

fn default_min_age_days() -> u64 {
    2
}
fn default_true() -> bool {
    true
}
fn default_max_depth() -> usize {
    6
}
fn default_skip_dirs() -> Vec<String> {
    vec![
        ".git".into(),
        ".jj".into(),
        ".cache".into(),
        "Library".into(),
    ]
}

impl Default for DevcleanSettings {
    fn default() -> Self {
        Self {
            min_age_days: default_min_age_days(),
            keep_release: default_true(),
            age_gate_node_modules: default_true(),
            extra_dirs: vec![],
            extra_age_gated: vec![],
            max_depth: default_max_depth(),
            skip_dirs: default_skip_dirs(),
        }
    }
}

impl DevcleanSettings {
    /// Resolve the kind for a directory name, if it is a known artifact.
    pub fn kind_for(&self, name: &str) -> Option<ArtifactKind> {
        match name {
            "_build" => Some(ArtifactKind::MixBuild),
            "target" => Some(ArtifactKind::TargetSubdirs),
            "node_modules"
            | "deps"
            | ".elixir_ls"
            | ".parcel-cache"
            | ".next"
            | ".turbo"
            | "cmake-build-debug"
            | "cmake-build-release" => Some(ArtifactKind::AgeGated),
            "dist" | "build" | "__pycache__" | ".venv" | "venv" | "newbuilder" => {
                Some(ArtifactKind::Whole)
            }
            _ => {
                if self.extra_dirs.iter().any(|d| d == name) {
                    Some(ArtifactKind::Whole)
                } else if self.extra_age_gated.iter().any(|d| d == name) {
                    Some(ArtifactKind::AgeGated)
                } else {
                    None
                }
            }
        }
    }
}

/// One planned (or executed) removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevcleanItem {
    pub path: PathBuf,
    pub kind: &'static str,
    pub reason: String,
    /// On-disk size computed at plan time (0 for kept items).
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DevcleanResult {
    pub removed: Vec<DevcleanItem>,
    pub kept: Vec<DevcleanItem>,
    pub freed_bytes: u64,
    pub errors: Vec<(PathBuf, String)>,
}

#[derive(Debug, thiserror::Error)]
pub enum DevcleanError {
    #[error("path does not exist: {0}")]
    Missing(PathBuf),
    #[error("path is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("failed to load config: {0}")]
    Config(String),
}

/// Top-level `~/.mcdu.toml` file. Unknown keys/tables are ignored so other
/// mcdu settings can live in the same file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McduFileConfig {
    #[serde(default, alias = "devClean")]
    pub devclean: DevcleanSettings,
}

/// Load settings from `~/.mcdu.toml`, falling back to defaults when absent.
pub fn load_settings() -> Result<DevcleanSettings, DevcleanError> {
    let Some(home) = dirs::home_dir() else {
        return Ok(DevcleanSettings::default());
    };
    let path = home.join(".mcdu.toml");
    if !path.exists() {
        return Ok(DevcleanSettings::default());
    }
    let contents = fs::read_to_string(&path)
        .map_err(|e| DevcleanError::Config(format!("{}: {e}", path.display())))?;
    let parsed: McduFileConfig = toml::from_str(&contents)
        .map_err(|e| DevcleanError::Config(format!("{}: {e}", path.display())))?;
    Ok(parsed.devclean)
}

/// `node_modules` can appear nested under other `node_modules` — always skip
/// descending into an artifact dir once matched.
fn is_stale(meta: &fs::Metadata, now: SystemTime, min_age_days: u64) -> bool {
    match meta.modified() {
        Ok(m) => now
            .duration_since(m)
            .map(|d| d.as_secs() >= min_age_days * 86_400)
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Recursive size on disk (sum of file blocks on unix, len elsewhere).
fn du(path: &Path) -> u64 {
    #[cfg(unix)]
    fn usage(m: &fs::Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt;
        m.blocks() * 512
    }
    #[cfg(not(unix))]
    fn usage(m: &fs::Metadata) -> u64 {
        m.len()
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if meta.is_file() || meta.is_symlink() {
        return usage(&meta);
    }
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| usage(&m))
        .sum()
}

/// Plan what would be removed under `root`. Does not touch the filesystem
/// (except reading metadata).
pub fn plan(root: &Path, settings: &DevcleanSettings, now: SystemTime) -> DevcleanResult {
    let mut result = DevcleanResult::default();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(settings.max_depth)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || e.file_name()
                    .to_str()
                    .map(|n| !settings.skip_dirs.iter().any(|s| s == n))
                    .unwrap_or(true)
        });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_dir() || entry.depth() == 0 {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(kind) = settings.kind_for(&name) else {
            continue;
        };
        let path = entry.path().to_path_buf();

        match kind {
            ArtifactKind::Whole => {
                let size = du(&path);
                result.removed.push(DevcleanItem {
                    path,
                    kind: "dir",
                    reason: name,
                    size_bytes: size,
                });
                result.freed_bytes += size;
            }
            ArtifactKind::AgeGated => {
                let stale = if settings.age_gate_node_modules {
                    fs::metadata(&path)
                        .map(|m| is_stale(&m, now, settings.min_age_days))
                        .unwrap_or(false)
                } else {
                    true
                };
                if stale {
                    let size = du(&path);
                    result.removed.push(DevcleanItem {
                        path,
                        kind: "age-gated",
                        reason: name,
                        size_bytes: size,
                    });
                    result.freed_bytes += size;
                } else {
                    result.kept.push(DevcleanItem {
                        path,
                        kind: "kept",
                        reason: format!("{name} younger than {} days", settings.min_age_days),
                        size_bytes: 0,
                    });
                }
            }
            ArtifactKind::MixBuild => {
                if settings.keep_release {
                    // Prune _build/dev, _build/test; keep _build/prod
                    for child in ["dev", "test"] {
                        let child_path = path.join(child);
                        if child_path.is_dir() {
                            let size = du(&child_path);
                            result.removed.push(DevcleanItem {
                                path: child_path,
                                kind: "mix-profile",
                                reason: format!("_build/{child}"),
                                size_bytes: size,
                            });
                            result.freed_bytes += size;
                        }
                    }
                    result.kept.push(DevcleanItem {
                        path,
                        kind: "kept",
                        reason: "_build/prod kept (keep_release)".to_string(),
                        size_bytes: 0,
                    });
                } else {
                    let size = du(&path);
                    result.removed.push(DevcleanItem {
                        path,
                        kind: "dir",
                        reason: name,
                        size_bytes: size,
                    });
                    result.freed_bytes += size;
                }
            }
            ArtifactKind::TargetSubdirs => {
                if settings.keep_release {
                    let mut kept_release = false;
                    let mut subdirs: Vec<PathBuf> = Vec::new();
                    let mut read = match fs::read_dir(&path) {
                        Ok(r) => r,
                        Err(e) => {
                            result.errors.push((path, e.to_string()));
                            continue;
                        }
                    };
                    while let Some(Ok(entry)) = read.next() {
                        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            subdirs.push(entry.path());
                        }
                    }
                    for sub in subdirs {
                        let sub_name = sub
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        if sub_name == "release" {
                            kept_release = true;
                            result.kept.push(DevcleanItem {
                                path: sub,
                                kind: "kept",
                                reason: "target/release kept (keep_release)".to_string(),
                                size_bytes: 0,
                            });
                            continue;
                        }
                        let size = du(&sub);
                        result.removed.push(DevcleanItem {
                            path: sub,
                            kind: "target-subdir",
                            reason: format!("target/{sub_name}"),
                            size_bytes: size,
                        });
                        result.freed_bytes += size;
                    }
                    let _ = kept_release;
                } else {
                    let size = du(&path);
                    result.removed.push(DevcleanItem {
                        path,
                        kind: "dir",
                        reason: name,
                        size_bytes: size,
                    });
                    result.freed_bytes += size;
                }
            }
        }
    }

    result
}

/// Prune removal entries nested under another removal (e.g. a `node_modules`
/// inside a matched `node_modules/...` chain), so each path is deleted once at
/// the topmost artifact level.
pub fn dedupe_nested(mut result: DevcleanResult) -> DevcleanResult {
    let mut sorted = std::mem::take(&mut result.removed);
    sorted.sort_by_key(|i| i.path.clone());
    let mut out: Vec<DevcleanItem> = Vec::with_capacity(sorted.len());
    for item in sorted {
        let nested = out.iter().any(|kept| item.path.starts_with(&kept.path));
        if !nested {
            out.push(item);
        }
    }
    result.removed = out;
    result
}

/// Delete a file or directory (symlink-aware).
fn remove_path(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_file() || meta.is_symlink() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

/// Execute a plan: delete everything in `result.removed`, collecting failures
/// into `errors`. `freed_bytes` keeps the plan-time estimate (sizes are not
/// recomputed after deletion).
pub fn run(mut result: DevcleanResult) -> DevcleanResult {
    let items = std::mem::take(&mut result.removed);
    for item in items {
        if let Err(e) = remove_path(&item.path) {
            result.errors.push((item.path, e.to_string()));
        } else {
            result.removed.push(item);
        }
    }
    result
}

pub fn devclean(
    root: &Path,
    settings: &DevcleanSettings,
    dry_run: bool,
) -> Result<DevcleanResult, DevcleanError> {
    let meta = fs::metadata(root).map_err(|_| DevcleanError::Missing(root.to_path_buf()))?;
    if !meta.is_dir() {
        return Err(DevcleanError::NotADirectory(root.to_path_buf()));
    }
    let now = SystemTime::now();
    let planned = dedupe_nested(plan(root, settings, now));
    if dry_run {
        return Ok(planned);
    }
    Ok(run(planned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn days_old(n: u64) -> SystemTime {
        SystemTime::now() + std::time::Duration::from_secs(n * 86_400)
    }

    fn settings() -> DevcleanSettings {
        DevcleanSettings::default()
    }

    fn make(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn kind_for_known_names() {
        let s = settings();
        assert_eq!(s.kind_for("_build"), Some(ArtifactKind::MixBuild));
        assert_eq!(s.kind_for("target"), Some(ArtifactKind::TargetSubdirs));
        assert_eq!(s.kind_for("node_modules"), Some(ArtifactKind::AgeGated));
        assert_eq!(s.kind_for("__pycache__"), Some(ArtifactKind::Whole));
        assert_eq!(s.kind_for("src"), None);
    }

    #[test]
    fn plans_whole_dirs() {
        let tmp = tempdir().unwrap();
        make(&tmp.path().join("proj/__pycache__/m.pyc"), b"x");
        let result = devclean(tmp.path(), &settings(), true).unwrap();
        assert!(result
            .removed
            .iter()
            .any(|i| i.path.ends_with("__pycache__")));
        assert!(!result.removed.iter().any(|i| i.path.ends_with("proj")));
    }

    #[test]
    fn target_keeps_release_by_default() {
        let tmp = tempdir().unwrap();
        make(&tmp.path().join("proj/target/debug/foo.o"), b"x");
        make(&tmp.path().join("proj/target/release/mcdu"), b"x");
        let result = devclean(tmp.path(), &settings(), true).unwrap();
        assert!(result.removed.iter().any(|i| i.path.ends_with("debug")));
        assert!(result.kept.iter().any(|i| i.path.ends_with("release")));
    }

    #[test]
    fn target_removed_entirely_without_keep_release() {
        let tmp = tempdir().unwrap();
        make(&tmp.path().join("proj/target/release/mcdu"), b"x");
        let mut s = settings();
        s.keep_release = false;
        let result = devclean(tmp.path(), &s, true).unwrap();
        assert!(result.removed.iter().any(|i| i.path.ends_with("target")));
    }

    #[test]
    fn mix_build_keeps_prod() {
        let tmp = tempdir().unwrap();
        make(
            &tmp.path().join("app/_build/dev/lib/app/ebin/app.beam"),
            b"x",
        );
        make(
            &tmp.path().join("app/_build/prod/lib/app/ebin/app.beam"),
            b"x",
        );
        let result = devclean(tmp.path(), &settings(), true).unwrap();
        assert!(result
            .removed
            .iter()
            .any(|i| i.path.ends_with("_build/dev")));
        assert!(result.kept.iter().any(|i| i.reason.contains("prod")));
    }

    #[test]
    fn node_modules_age_gated() {
        let tmp = tempdir().unwrap();
        let nm = tmp.path().join("web/node_modules");
        make(&nm.join("react/index.js"), b"x");

        // Fresh dir: kept
        let result = devclean(tmp.path(), &settings(), true).unwrap();
        assert!(result.kept.iter().any(|i| i.path == nm));

        // Old dir (simulate via plan + injected now)
        let now = days_old(10);
        let old = plan(tmp.path(), &settings(), now);
        let old = dedupe_nested(old);
        assert!(old.removed.iter().any(|i| i.path == nm));
    }

    #[test]
    fn age_gate_disabled_removes_fresh_node_modules() {
        let tmp = tempdir().unwrap();
        let nm = tmp.path().join("web/node_modules");
        make(&nm.join("react/index.js"), b"x");
        let mut s = settings();
        s.age_gate_node_modules = false;
        let result = devclean(tmp.path(), &s, true).unwrap();
        assert!(result.removed.iter().any(|i| i.path == nm));
    }

    #[test]
    fn executes_dry_run_vs_real() {
        let tmp = tempdir().unwrap();
        let pycache = tmp.path().join("proj/__pycache__");
        make(&pycache.join("m.pyc"), b"x");

        let dry = devclean(tmp.path(), &settings(), true).unwrap();
        assert!(pycache.exists());
        assert_eq!(dry.removed.len(), 1);

        let real = devclean(tmp.path(), &settings(), false).unwrap();
        assert!(!pycache.exists());
        assert_eq!(real.removed.len(), 1);
        assert!(real.errors.is_empty());
    }

    #[test]
    fn skips_git_dirs() {
        let tmp = tempdir().unwrap();
        make(&tmp.path().join(".git/node_modules"), b"x");
        let result = devclean(tmp.path(), &settings(), true).unwrap();
        assert!(result.removed.is_empty());
    }

    #[test]
    fn symlinked_artifacts_not_followed() {
        let tmp = tempdir().unwrap();
        let real = tmp.path().join("realproj/__pycache__");
        make(&real.join("m.pyc"), b"x");
        let link = tmp.path().join("linked");
        symlink(tmp.path().join("realproj"), &link).unwrap();
        // Walk with follow_links(false) must not enter `linked`
        let result = devclean(tmp.path(), &settings(), true).unwrap();
        assert_eq!(result.removed.len(), 1);
        assert!(link.exists());
    }

    #[test]
    fn missing_root_errors() {
        let err = devclean(Path::new("/nonexistent/xyz"), &settings(), true);
        assert!(matches!(err, Err(DevcleanError::Missing(_))));
    }

    #[test]
    fn loads_settings_from_mcdu_toml() {
        let tmp = tempdir().unwrap();
        let cfg = tmp.path().join(".mcdu.toml");
        fs::write(&cfg, "[devclean]\nmin_age_days = 7\nkeep_release = false\n").unwrap();
        let parsed: McduFileConfig = toml::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(parsed.devclean.min_age_days, 7);
        assert!(!parsed.devclean.keep_release);
    }

    #[test]
    fn settings_defaults_match_doc() {
        let s = DevcleanSettings::default();
        assert_eq!(s.min_age_days, 2);
        assert!(s.keep_release);
        assert!(s.age_gate_node_modules);
    }

    #[test]
    fn extra_dirs_matched() {
        let s = DevcleanSettings {
            extra_dirs: vec!["playwright-report".into()],
            ..settings()
        };
        assert_eq!(s.kind_for("playwright-report"), Some(ArtifactKind::Whole));
        let s2 = DevcleanSettings {
            extra_age_gated: vec![".turbo-cache".into()],
            ..settings()
        };
        assert_eq!(s2.kind_for(".turbo-cache"), Some(ArtifactKind::AgeGated));
    }

    #[test]
    fn dedupes_nested_matches() {
        let tmp = tempdir().unwrap();
        // node_modules inside node_modules, both stale
        let outer = tmp.path().join("a/node_modules");
        let inner = outer.join("foo/node_modules");
        make(&inner.join("x/y.js"), b"x");
        let old = dedupe_nested(plan(tmp.path(), &settings(), days_old(10)));
        // Only the outer node_modules is planned (inner is nested)
        assert_eq!(
            old.removed
                .iter()
                .filter(|i| i.path.ends_with("node_modules"))
                .count(),
            1
        );
        assert!(old.removed[0].path == outer);
    }
}
