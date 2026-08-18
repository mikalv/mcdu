use crate::git;
use crate::quarantine::{default_quarantine, QuarantineError};
use crate::rules::Candidate;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

#[derive(Debug, Clone, Copy)]
pub enum CleanupStage {
    Files,
    Git,
    Command,
}

#[derive(Debug, Clone)]
pub struct CleanupProgress {
    pub path: PathBuf,
    pub current: u64,
    pub total: u64,
    pub freed_bytes: u64,
    pub stage: CleanupStage,
}

#[derive(Debug, Default, Clone)]
pub struct CleanupResult {
    pub freed_bytes: u64,
    pub errors: Vec<(PathBuf, String)>,
    /// Paths successfully deleted, quarantined, or cleaned via cleanup_command.
    pub deleted_paths: Vec<PathBuf>,
}

pub fn execute_async(
    candidates: Vec<Candidate>,
    run_git: bool,
    git_roots: Vec<PathBuf>,
    progress_tx: Option<mpsc::Sender<CleanupProgress>>,
) -> thread::JoinHandle<CleanupResult> {
    thread::spawn(move || execute(candidates, run_git, git_roots, progress_tx))
}

pub fn execute(
    candidates: Vec<Candidate>,
    run_git: bool,
    git_roots: Vec<PathBuf>,
    progress_tx: Option<mpsc::Sender<CleanupProgress>>,
) -> CleanupResult {
    let total = candidates.len() as u64;
    let mut result = CleanupResult::default();
    let quarantine = default_quarantine();

    for (idx, candidate) in candidates.into_iter().enumerate() {
        let path = candidate.path.clone();
        let size = candidate.size_bytes;

        let (outcome, stage) = if let Some(ref cmd) = candidate.cleanup_command {
            (
                run_cleanup_command(cmd, &path, candidate.is_directory),
                CleanupStage::Command,
            )
        } else if quarantine.should_skip(&candidate.rule_category) {
            (delete_path(&path).map(|_| ()), CleanupStage::Files)
        } else {
            let q = match quarantine.quarantine(vec![(
                path.clone(),
                candidate.rule_category.clone(),
                candidate.rule_name.clone(),
                size,
            )]) {
                Ok(_) => Ok(()),
                Err(QuarantineError::PartialFailure {
                    succeeded, message, ..
                }) => {
                    if succeeded >= 1 {
                        Ok(())
                    } else {
                        Err(std::io::Error::other(message))
                    }
                }
                Err(e) => Err(std::io::Error::other(e.to_string())),
            };
            (q, CleanupStage::Files)
        };

        match outcome {
            Ok(()) => {
                result.freed_bytes += size;
                result.deleted_paths.push(path.clone());
            }
            Err(err) => {
                result.errors.push((path.clone(), err.to_string()));
            }
        }

        if let Some(tx) = &progress_tx {
            let _ = tx.send(CleanupProgress {
                path,
                current: (idx as u64) + 1,
                total,
                freed_bytes: result.freed_bytes,
                stage,
            });
        }
    }

    if run_git {
        let repos = git::find_git_repos(&git_roots);
        let total_git = repos.len() as u64;
        for (idx, repo) in repos.into_iter().enumerate() {
            if let Err(err) = git::run_git_gc(&repo) {
                result
                    .errors
                    .push((repo.clone(), format!("git gc failed: {err}")));
            }
            if let Some(tx) = &progress_tx {
                let _ = tx.send(CleanupProgress {
                    path: repo,
                    current: (idx as u64) + 1,
                    total: total_git,
                    freed_bytes: result.freed_bytes,
                    stage: CleanupStage::Git,
                });
            }
        }
    }

    result
}

pub fn dry_run(candidates: Vec<Candidate>) -> CleanupResult {
    let total: u64 = candidates.iter().map(|c| c.size_bytes).sum();
    CleanupResult {
        freed_bytes: total,
        errors: Vec::new(),
        deleted_paths: Vec::new(),
    }
}

/// Expand `{path}` / `{dir}` and run via `sh -c`.
///
/// - `{path}` — absolute candidate path
/// - `{dir}` — directory to operate in (the path if it's a dir, else its parent)
///
/// Working directory is set to `{dir}`.
pub fn run_cleanup_command(template: &str, path: &Path, is_directory: bool) -> std::io::Result<()> {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let dir = if is_directory || abs.is_dir() {
        abs.clone()
    } else {
        abs.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let cmd = template
        .replace("{path}", &abs.to_string_lossy())
        .replace("{dir}", &dir.to_string_lossy());

    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&dir)
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit {}", output.status)
        };
        Err(std::io::Error::other(format!(
            "cleanup_command failed: {detail}"
        )))
    }
}

/// Delete a path using symlink-aware metadata (never follow dangling symlinks into remove_dir_all).
fn delete_path(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let ft = meta.file_type();
    if ft.is_file() || ft.is_symlink() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn candidate(path: PathBuf, size: u64) -> Candidate {
        Candidate::new(
            path,
            "rule".into(),
            "category".into(),
            "**/*".into(),
            size,
            None,
            false,
        )
    }

    #[test]
    fn deletes_files_and_reports_progress() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let file_a = tmp.path().join("a.txt");
        let file_b = tmp.path().join("b.txt");
        let mut fa = std::fs::File::create(&file_a).unwrap();
        writeln!(fa, "hello").unwrap();
        let mut fb = std::fs::File::create(&file_b).unwrap();
        writeln!(fb, "world!").unwrap();

        let candidates = vec![candidate(file_a.clone(), 6), candidate(file_b.clone(), 7)];
        let (tx, rx) = mpsc::channel();
        let result = execute(candidates, false, Vec::new(), Some(tx));

        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
        assert!(!file_a.exists());
        assert!(!file_b.exists());
        assert_eq!(result.freed_bytes, 13);
        assert_eq!(result.deleted_paths.len(), 2);

        let progress: Vec<_> = rx.try_iter().collect();
        assert_eq!(progress.len(), 2);
        assert_eq!(progress.last().unwrap().current, 2);
    }

    #[test]
    fn continues_on_errors() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let missing = tmp.path().join("missing.txt");
        let existing = tmp.path().join("exists.txt");
        std::fs::write(&existing, "hi").unwrap();

        let candidates = vec![
            candidate(missing.clone(), 5),
            candidate(existing.clone(), 2),
        ];

        let result = execute(candidates, false, Vec::new(), None);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.freed_bytes, 2);
        assert!(!existing.exists());
    }

    #[test]
    fn deletes_symlinks_without_following() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let target = tmp.path().join("target.txt");
        std::fs::write(&target, "keep").unwrap();
        let link = tmp.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = execute(vec![candidate(link.clone(), 4)], false, Vec::new(), None);
        assert!(result.errors.is_empty());
        assert!(!link.exists());
        assert!(target.exists());
    }

    #[test]
    fn skip_category_deletes_permanently() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let file = tmp.path().join("cache.bin");
        std::fs::write(&file, "x").unwrap();
        let mut c = candidate(file.clone(), 1);
        c.rule_category = "Browser Caches".into();

        let result = execute(vec![c], false, Vec::new(), None);
        assert!(result.errors.is_empty());
        assert!(!file.exists());
        let q = default_quarantine();
        assert!(q.list().unwrap().is_empty());
    }

    #[test]
    fn cleanup_command_runs_instead_of_delete() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let dir = tmp.path().join("proj");
        fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk.txt");
        fs::write(&junk, "bye").unwrap();

        let mut c = candidate(dir.clone(), 3);
        c.is_directory = true;
        // Clear contents but leave the directory (like cargo clean)
        c.cleanup_command = Some("rm -f '{path}/junk.txt'".into());

        let result = execute(vec![c], false, Vec::new(), None);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert!(!junk.exists());
        assert!(dir.exists());
        assert_eq!(result.deleted_paths.len(), 1);
        // Should not have quarantined
        assert!(default_quarantine().list().unwrap().is_empty());
    }

    #[test]
    fn cleanup_command_failure_is_reported() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());

        let dir = tmp.path().join("proj");
        fs::create_dir_all(&dir).unwrap();

        let mut c = candidate(dir.clone(), 1);
        c.is_directory = true;
        c.cleanup_command = Some("exit 42".into());

        let result = execute(vec![c], false, Vec::new(), None);
        assert_eq!(result.errors.len(), 1);
        assert!(result.deleted_paths.is_empty());
        assert!(dir.exists());
    }

    #[test]
    fn dry_run_sums_sizes_without_deleting() {
        let tmp = tempdir().unwrap();
        let file_a = tmp.path().join("a.txt");
        std::fs::write(&file_a, "hello").unwrap();
        let metadata = std::fs::metadata(&file_a).unwrap();
        let size = metadata.len();
        let candidates = vec![candidate(file_a.clone(), size)];
        let result = dry_run(candidates);
        assert_eq!(result.freed_bytes, size);
        assert!(file_a.exists());
    }
}
