//! Quarantine system for safe deletion with undo capability
//!
//! Instead of permanently deleting files, moves them to a quarantine directory
//! with metadata for restoration. Auto-purges after configurable TTL.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use thiserror::Error;
use uuid::Uuid;

/// Default quarantine directory name under base_dir
const QUARANTINE_SUBDIR: &str = "quarantine";

/// Default time-to-live for quarantined items (7 days)
const DEFAULT_TTL_DAYS: u64 = 7;

/// Default maximum quarantine size (10 GB)
const DEFAULT_MAX_SIZE_GB: u64 = 10;

#[derive(Error, Debug)]
pub enum QuarantineError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Quarantine entry not found: {0}")]
    NotFound(String),

    #[error("Original path already exists: {0}")]
    PathExists(PathBuf),

    #[error("Quarantine size limit exceeded")]
    SizeLimitExceeded,

    #[error("Partial quarantine failure after {succeeded} items: {message}")]
    PartialFailure {
        succeeded: usize,
        message: String,
        manifest_id: String,
    },
}

/// A single quarantined item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineItem {
    /// Original absolute path
    pub original_path: PathBuf,
    /// Relative path within quarantine data directory
    pub quarantine_path: String,
    /// Size in bytes
    pub size_bytes: u64,
    /// Category from cleanup rule
    pub category: String,
    /// Rule name that matched this item
    pub rule_name: String,
}

/// Manifest for a quarantine batch (one deletion operation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineManifest {
    /// Unique identifier
    pub id: String,
    /// When the quarantine was created
    pub timestamp: SystemTime,
    /// Items in this batch
    pub items: Vec<QuarantineItem>,
    /// Total size of all items
    pub total_size_bytes: u64,
    /// When this quarantine expires
    pub expires_at: SystemTime,
    /// Whether items can still be restored
    pub can_restore: bool,
}

/// Quarantine settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineSettings {
    /// Time-to-live in days
    pub ttl_days: u64,
    /// Maximum total quarantine size in GB
    pub max_size_gb: u64,
    /// Categories to skip quarantine (delete directly)
    pub skip_categories: Vec<String>,
}

impl Default for QuarantineSettings {
    fn default() -> Self {
        Self {
            ttl_days: DEFAULT_TTL_DAYS,
            max_size_gb: DEFAULT_MAX_SIZE_GB,
            skip_categories: vec!["Browser Caches".to_string(), "IDE Caches".to_string()],
        }
    }
}

/// Quarantine manager
pub struct Quarantine {
    /// Base directory (e.g. ~/.mcdu)
    base_dir: PathBuf,
    /// Settings
    settings: QuarantineSettings,
}

/// Default quarantine under `$HOME/.mcdu` (or `dirs::home_dir`).
pub fn default_quarantine() -> Quarantine {
    let base = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mcdu");
    Quarantine::new(base, QuarantineSettings::default())
}

impl Quarantine {
    /// Create a new quarantine manager
    pub fn new(base_dir: PathBuf, settings: QuarantineSettings) -> Self {
        Self { base_dir, settings }
    }

    /// Get the quarantine directory
    fn quarantine_dir(&self) -> PathBuf {
        self.base_dir.join(QUARANTINE_SUBDIR)
    }

    /// Ensure quarantine directory exists
    fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(self.quarantine_dir())
    }

    /// Check if a category should skip quarantine
    pub fn should_skip(&self, category: &str) -> bool {
        self.settings.skip_categories.iter().any(|c| c == category)
    }

    fn write_manifest(&self, batch_dir: &Path, manifest: &QuarantineManifest) -> Result<(), QuarantineError> {
        let manifest_path = batch_dir.join("manifest.json");
        let tmp_path = batch_dir.join("manifest.json.tmp");
        let manifest_json = serde_json::to_string_pretty(manifest)?;
        fs::write(&tmp_path, &manifest_json)?;
        fs::rename(&tmp_path, &manifest_path)?;
        Ok(())
    }

    /// Get current quarantine size in bytes
    pub fn current_size(&self) -> io::Result<u64> {
        let mut total = 0u64;
        let qdir = self.quarantine_dir();

        if !qdir.exists() {
            return Ok(0);
        }

        for entry in fs::read_dir(&qdir)? {
            let entry = entry?;
            let manifest_path = entry.path().join("manifest.json");
            if manifest_path.exists() {
                if let Ok(content) = fs::read_to_string(&manifest_path) {
                    if let Ok(manifest) = serde_json::from_str::<QuarantineManifest>(&content) {
                        total += manifest.total_size_bytes;
                    }
                }
            }
        }

        Ok(total)
    }

    /// List all quarantine entries
    pub fn list(&self) -> io::Result<Vec<QuarantineManifest>> {
        let mut manifests = Vec::new();
        let qdir = self.quarantine_dir();

        if !qdir.exists() {
            return Ok(manifests);
        }

        for entry in fs::read_dir(&qdir)? {
            let entry = entry?;
            let manifest_path = entry.path().join("manifest.json");
            if manifest_path.exists() {
                if let Ok(content) = fs::read_to_string(&manifest_path) {
                    if let Ok(manifest) = serde_json::from_str::<QuarantineManifest>(&content) {
                        manifests.push(manifest);
                    }
                }
            }
        }

        // Sort by timestamp, newest first
        manifests.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(manifests)
    }

    /// Quarantine items (move to quarantine directory).
    /// Writes a staging manifest first and updates it after each successful move
    /// so partial batches remain visible to `list()` / `restore()`.
    pub fn quarantine(
        &self,
        items: Vec<(PathBuf, String, String, u64)>,
    ) -> Result<QuarantineManifest, QuarantineError> {
        self.ensure_dir()?;

        // Check size limit
        let current_size = self.current_size()?;
        let new_size: u64 = items.iter().map(|(_, _, _, size)| size).sum();
        let max_bytes = self.settings.max_size_gb * 1024 * 1024 * 1024;

        if current_size + new_size > max_bytes {
            self.purge_expired()?;
            let current_size = self.current_size()?;
            if current_size + new_size > max_bytes {
                return Err(QuarantineError::SizeLimitExceeded);
            }
        }

        let id = Uuid::new_v4().to_string();
        let now = SystemTime::now();
        let expires_at = now + Duration::from_secs(self.settings.ttl_days * 24 * 3600);

        let batch_dir = self.quarantine_dir().join(&id);
        let data_dir = batch_dir.join("data");
        fs::create_dir_all(&data_dir)?;

        let mut manifest = QuarantineManifest {
            id: id.clone(),
            timestamp: now,
            items: Vec::new(),
            total_size_bytes: 0,
            expires_at,
            can_restore: true,
        };
        // Staging manifest so the batch is always listable
        self.write_manifest(&batch_dir, &manifest)?;

        for (idx, (path, category, rule_name, size)) in items.into_iter().enumerate() {
            let quarantine_path = format!("{}", idx);
            let dest = data_dir.join(&quarantine_path);

            match move_path(&path, &dest) {
                Ok(()) => {
                    manifest.items.push(QuarantineItem {
                        original_path: path,
                        quarantine_path,
                        size_bytes: size,
                        category,
                        rule_name,
                    });
                    manifest.total_size_bytes += size;
                    self.write_manifest(&batch_dir, &manifest)?;
                }
                Err(e) => {
                    // Keep partial batch listable
                    self.write_manifest(&batch_dir, &manifest)?;
                    return Err(QuarantineError::PartialFailure {
                        succeeded: manifest.items.len(),
                        message: format!("{}: {}", path.display(), e),
                        manifest_id: id,
                    });
                }
            }
        }

        Ok(manifest)
    }

    /// Restore a quarantine entry. Updates the manifest after each successful
    /// item so a mid-batch PathExists does not permanently lock the batch.
    pub fn restore(&self, id: &str) -> Result<Vec<PathBuf>, QuarantineError> {
        let batch_dir = self.quarantine_dir().join(id);
        let manifest_path = batch_dir.join("manifest.json");

        if !manifest_path.exists() {
            return Err(QuarantineError::NotFound(id.to_string()));
        }

        let content = fs::read_to_string(&manifest_path)?;
        let mut manifest: QuarantineManifest = serde_json::from_str(&content)?;

        if !manifest.can_restore {
            return Err(QuarantineError::NotFound(id.to_string()));
        }

        let data_dir = batch_dir.join("data");
        let mut restored = Vec::new();

        while !manifest.items.is_empty() {
            let item = manifest.items.remove(0);
            let source = data_dir.join(&item.quarantine_path);

            if !path_exists_nofollow(&source) {
                manifest.total_size_bytes = manifest.items.iter().map(|i| i.size_bytes).sum();
                self.write_manifest(&batch_dir, &manifest)?;
                continue;
            }

            let dest = item.original_path.clone();
            if path_exists_nofollow(&dest) {
                manifest.items.insert(0, item);
                manifest.total_size_bytes = manifest.items.iter().map(|i| i.size_bytes).sum();
                self.write_manifest(&batch_dir, &manifest)?;
                return Err(QuarantineError::PathExists(dest));
            }

            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }

            move_path(&source, &dest)?;
            restored.push(dest);
            manifest.total_size_bytes = manifest.items.iter().map(|i| i.size_bytes).sum();
            self.write_manifest(&batch_dir, &manifest)?;
        }

        let _ = fs::remove_dir_all(&batch_dir);
        Ok(restored)
    }

    /// Permanently delete a quarantine entry
    pub fn purge(&self, id: &str) -> Result<u64, QuarantineError> {
        let batch_dir = self.quarantine_dir().join(id);
        let manifest_path = batch_dir.join("manifest.json");

        if !manifest_path.exists() {
            return Err(QuarantineError::NotFound(id.to_string()));
        }

        let content = fs::read_to_string(&manifest_path)?;
        let manifest: QuarantineManifest = serde_json::from_str(&content)?;

        let size = manifest.total_size_bytes;
        fs::remove_dir_all(&batch_dir)?;

        Ok(size)
    }

    /// Purge all expired entries
    pub fn purge_expired(&self) -> Result<u64, QuarantineError> {
        let now = SystemTime::now();
        let manifests = self.list()?;
        let mut purged_size = 0u64;

        for manifest in manifests {
            if manifest.expires_at <= now {
                purged_size += self.purge(&manifest.id)?;
            }
        }

        Ok(purged_size)
    }

    /// Purge all entries
    pub fn purge_all(&self) -> Result<u64, QuarantineError> {
        let manifests = self.list()?;
        let mut purged_size = 0u64;

        for manifest in manifests {
            purged_size += self.purge(&manifest.id)?;
        }

        Ok(purged_size)
    }

    /// Get quarantine statistics
    pub fn stats(&self) -> io::Result<QuarantineStats> {
        let manifests = self.list()?;
        let now = SystemTime::now();

        let total_entries = manifests.len();
        let total_items: usize = manifests.iter().map(|m| m.items.len()).sum();
        let total_size: u64 = manifests.iter().map(|m| m.total_size_bytes).sum();
        let expired_count = manifests.iter().filter(|m| m.expires_at <= now).count();

        let oldest = manifests.last().map(|m| m.timestamp);
        let newest = manifests.first().map(|m| m.timestamp);

        Ok(QuarantineStats {
            total_entries,
            total_items,
            total_size_bytes: total_size,
            expired_count,
            oldest_entry: oldest,
            newest_entry: newest,
            max_size_bytes: self.settings.max_size_gb * 1024 * 1024 * 1024,
        })
    }
}

/// Quarantine statistics
#[derive(Debug, Clone)]
pub struct QuarantineStats {
    pub total_entries: usize,
    pub total_items: usize,
    pub total_size_bytes: u64,
    pub expired_count: usize,
    pub oldest_entry: Option<SystemTime>,
    pub newest_entry: Option<SystemTime>,
    pub max_size_bytes: u64,
}

fn path_exists_nofollow(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Move a file, symlink, or directory to dest (rename, or copy+delete cross-device).
fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }

    let meta = fs::symlink_metadata(src)?;
    let ft = meta.file_type();

    if ft.is_symlink() {
        let target = fs::read_link(src)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, dst)?;
        #[cfg(windows)]
        {
            if target.exists() && target.is_dir() {
                std::os::windows::fs::symlink_dir(&target, dst)?;
            } else {
                std::os::windows::fs::symlink_file(&target, dst)?;
            }
        }
        fs::remove_file(src)?;
    } else if ft.is_dir() {
        copy_dir_all(src, dst)?;
        fs::remove_dir_all(src)?;
    } else {
        fs::copy(src, dst)?;
        fs::remove_file(src)?;
    }
    Ok(())
}

/// Recursively copy a directory, preserving symlinks.
fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    if let Ok(meta) = fs::symlink_metadata(src) {
        let _ = fs::set_permissions(dst, meta.permissions());
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_symlink() {
            let target = fs::read_link(&src_path)?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &dst_path)?;
            #[cfg(windows)]
            {
                if target.exists() && target.is_dir() {
                    std::os::windows::fs::symlink_dir(&target, &dst_path)?;
                } else {
                    std::os::windows::fs::symlink_file(&target, &dst_path)?;
                }
            }
        } else if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn quarantine_and_restore() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");

        fs::create_dir_all(&source_dir).unwrap();
        let test_file = source_dir.join("test.txt");
        fs::write(&test_file, "hello world").unwrap();

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());

        let manifest = quarantine
            .quarantine(vec![(
                test_file.clone(),
                "Test".to_string(),
                "test-rule".to_string(),
                11,
            )])
            .unwrap();

        assert_eq!(manifest.items.len(), 1);
        assert!(!test_file.exists());

        let restored = quarantine.restore(&manifest.id).unwrap();
        assert_eq!(restored.len(), 1);
        assert!(test_file.exists());
        assert_eq!(fs::read_to_string(&test_file).unwrap(), "hello world");
    }

    #[test]
    fn quarantine_directory() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");

        let target_dir = source_dir.join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("a.txt"), "a").unwrap();
        fs::write(target_dir.join("b.txt"), "b").unwrap();

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());

        let manifest = quarantine
            .quarantine(vec![(
                target_dir.clone(),
                "Rust".to_string(),
                "target".to_string(),
                2,
            )])
            .unwrap();

        assert!(!target_dir.exists());

        quarantine.restore(&manifest.id).unwrap();
        assert!(target_dir.exists());
        assert!(target_dir.join("a.txt").exists());
        assert!(target_dir.join("b.txt").exists());
    }

    #[test]
    fn list_and_purge() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());

        for i in 0..3 {
            let file = source_dir.join(format!("file{}.txt", i));
            fs::write(&file, format!("content {}", i)).unwrap();
            quarantine
                .quarantine(vec![(file, "Test".to_string(), "test".to_string(), 10)])
                .unwrap();
        }

        let list = quarantine.list().unwrap();
        assert_eq!(list.len(), 3);

        quarantine.purge(&list[0].id).unwrap();

        let list = quarantine.list().unwrap();
        assert_eq!(list.len(), 2);

        quarantine.purge_all().unwrap();

        let list = quarantine.list().unwrap();
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn stats() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());

        let file = source_dir.join("test.txt");
        fs::write(&file, "test content").unwrap();
        quarantine
            .quarantine(vec![(file, "Test".to_string(), "test".to_string(), 12)])
            .unwrap();

        let stats = quarantine.stats().unwrap();
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.total_items, 1);
        assert_eq!(stats.total_size_bytes, 12);
        assert_eq!(stats.expired_count, 0);
    }

    #[test]
    fn partial_failure_leaves_listable_batch() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let good = source_dir.join("good.txt");
        fs::write(&good, "ok").unwrap();
        let missing = source_dir.join("missing.txt");

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());
        let err = quarantine
            .quarantine(vec![
                (good.clone(), "Test".into(), "t".into(), 2),
                (missing, "Test".into(), "t".into(), 1),
            ])
            .unwrap_err();

        match err {
            QuarantineError::PartialFailure {
                succeeded,
                manifest_id,
                ..
            } => {
                assert_eq!(succeeded, 1);
                let list = quarantine.list().unwrap();
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].id, manifest_id);
                assert_eq!(list[0].items.len(), 1);
                assert!(!good.exists());
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn dangling_symlink_quarantine_and_restore() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let link = source_dir.join("dangling");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/nonexistent/target", &link).unwrap();

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());
        let manifest = quarantine
            .quarantine(vec![(link.clone(), "Test".into(), "t".into(), 0)])
            .unwrap();
        assert!(!path_exists_nofollow(&link));

        quarantine.restore(&manifest.id).unwrap();
        assert!(path_exists_nofollow(&link));
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }

    #[test]
    fn restore_path_exists_then_retry() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join(".mcdu");
        let source_dir = tmp.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();

        let a = source_dir.join("a.txt");
        let b = source_dir.join("b.txt");
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();

        let quarantine = Quarantine::new(base_dir, QuarantineSettings::default());
        let manifest = quarantine
            .quarantine(vec![
                (a.clone(), "Test".into(), "t".into(), 1),
                (b.clone(), "Test".into(), "t".into(), 1),
            ])
            .unwrap();

        // Block restore of first item
        fs::write(&a, "conflict").unwrap();

        let err = quarantine.restore(&manifest.id).unwrap_err();
        assert!(matches!(err, QuarantineError::PathExists(_)));

        // Batch still listable with both items (first blocked)
        let list = quarantine.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].items.len(), 2);

        fs::remove_file(&a).unwrap();
        let restored = quarantine.restore(&manifest.id).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(a.exists());
        assert!(b.exists());
        assert!(quarantine.list().unwrap().is_empty());
    }
}
