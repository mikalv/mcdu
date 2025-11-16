use crate::cache::SizeCache;
use crate::changes::{get_fingerprint_path, DirectoryFingerprint, SizeChange};
use crate::delete;
use crate::logger;
use crate::modal::Modal;
use crate::platform::{self, DiskSpace};
use crate::scan;
use crate::scan::DirEntry;
use chrono::Local;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Browsing,
    Deleting,
    DryRun,
}

pub struct DeleteProgress {
    pub deleted_bytes: u64,
    pub total_bytes: u64,
    pub deleted_files: u64,
    pub total_files: u64,
    pub current_file: String,
    pub status: String,
}

pub struct App {
    pub _root_path: PathBuf,
    pub current_path: PathBuf,
    pub entries: Vec<DirEntry>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub mode: AppMode,
    pub modal: Option<Modal>,
    pub delete_progress: Option<DeleteProgress>,
    pub delete_thread: Option<JoinHandle<Result<(), String>>>,
    pub delete_rx: Option<mpsc::Receiver<DeleteProgressUpdate>>,
    pub notification: Option<String>,
    pub notification_time: Option<Instant>,
    pub show_help: bool,
    // Async scanning
    pub scan_thread: Option<JoinHandle<()>>,
    pub scan_rx: Option<mpsc::Receiver<ScanResult>>,
    pub is_scanning: bool,
    pub scanning_name: Option<String>,
    pub scan_progress: Option<(usize, usize)>, // (scanned, total)
    // Size cache for performance
    pub size_cache: SizeCache,
    // Disk space info
    pub disk_space: Option<DiskSpace>,
}

pub enum ScanResult {
    Progress {
        current_name: String,
        scanned_count: usize,
        total_count: usize,
    },
    Success(Vec<DirEntry>),
    Error(String),
}

pub enum DeleteProgressUpdate {
    // Progress updates during deletion (future enhancement)
    #[allow(dead_code)]
    Progress {
        bytes_done: u64,
        bytes_total: u64,
        files_done: u64,
        files_total: u64,
        current_file: String,
    },
    Complete {
        total_bytes: u64,
        total_files: u64,
    },
    Error(String),
}

impl App {
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self::new_with_root(root)
    }

    pub fn new_with_root(root: PathBuf) -> Self {
        let disk_space = platform::get_disk_space(&root);

        let mut app = App {
            _root_path: root.clone(),
            current_path: root.clone(),
            entries: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            mode: AppMode::Browsing,
            modal: None,
            delete_progress: None,
            delete_thread: None,
            delete_rx: None,
            notification: None,
            notification_time: None,
            show_help: false,
            scan_thread: None,
            scan_rx: None,
            is_scanning: false,
            scanning_name: None,
            scan_progress: None,
            size_cache: SizeCache::new(),
            disk_space,
        };
        app.refresh();
        app
    }

    pub fn select_next(&mut self) {
        if !self.entries.is_empty() && self.selected_index < self.entries.len() - 1 {
            self.selected_index += 1;
        }
    }

    pub fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn adjust_scroll(&mut self, viewport_height: usize) {
        // Ensure selected item is visible within viewport
        // Leave 2 lines for header (Path line + blank line)
        let usable_height = viewport_height.saturating_sub(2);

        if usable_height == 0 {
            return;
        }

        // If selected is above visible area, scroll up
        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }

        // If selected is below visible area, scroll down
        if self.selected_index >= self.scroll_offset + usable_height {
            self.scroll_offset = self.selected_index.saturating_sub(usable_height - 1);
        }
    }

    pub fn enter_directory(&mut self) {
        if let Some(entry) = self.entries.get(self.selected_index) {
            if entry.is_dir {
                self.current_path = entry.path.clone();
                self.selected_index = 0;
                self.scroll_offset = 0;
                self.disk_space = platform::get_disk_space(&self.current_path);
                self.refresh();
            }
        }
    }

    pub fn go_parent(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            if parent != self.current_path {
                self.current_path = parent.to_path_buf();
                self.selected_index = 0;
                self.scroll_offset = 0;
                self.disk_space = platform::get_disk_space(&self.current_path);
                self.refresh();
            }
        }
    }

    pub fn refresh(&mut self) {
        // Cancel any existing scan
        if let Some(thread) = self.scan_thread.take() {
            let _ = thread.join();
        }
        self.scan_rx = None;

        // Start async scan
        let path = self.current_path.clone();
        let cache = self.size_cache.clone();
        let (tx, rx) = mpsc::channel();

        let tx_clone = tx.clone();
        let handle = thread::spawn(move || {
            let result = match scan::scan_directory(&path, &cache, Some(&tx_clone)) {
                Ok(entries) => ScanResult::Success(entries),
                Err(e) => ScanResult::Error(e.to_string()),
            };
            let _ = tx_clone.send(result);
        });

        self.scan_thread = Some(handle);
        self.scan_rx = Some(rx);
        self.is_scanning = true;
    }

    pub fn hard_refresh(&mut self) {
        // Clear cache and refresh
        self.size_cache.clear();
        self.notification = Some("✓ Cache cleared - rescanning...".to_string());
        self.notification_time = Some(Instant::now());
        self.refresh();
    }

    pub fn update_scan_progress(&mut self) {
        if let Some(rx) = self.scan_rx.as_ref() {
            // Drain all available messages
            loop {
                match rx.try_recv() {
                    Ok(result) => {
                        match result {
                            ScanResult::Progress {
                                current_name,
                                scanned_count,
                                total_count,
                            } => {
                                // Update current scanning info
                                self.scanning_name = Some(current_name);
                                self.scan_progress = Some((scanned_count, total_count));
                            }
                            ScanResult::Success(mut entries) => {
                                self.is_scanning = false;
                                self.scan_thread = None;
                                self.scan_rx = None;
                                self.scanning_name = None;
                                self.scan_progress = None;

                                // Load previous fingerprint and detect changes
                                let fp_path = get_fingerprint_path(&self.current_path);

                                // Create fingerprint from scanned entries (without re-reading metadata)
                                let new_fp = DirectoryFingerprint::from_entries(&entries);
                                if let Ok(old_fp) = DirectoryFingerprint::load(&fp_path) {
                                    let changes = old_fp.get_changes(&new_fp);

                                    apply_size_changes(&mut entries, &changes);
                                }

                                // Save new fingerprint for next run
                                let _ = std::fs::create_dir_all(fp_path.parent().unwrap());
                                let _ = new_fp.save(&fp_path);

                                // Always keep the list size-sorted for easier scanning
                                entries.sort_by(|a, b| b.size.cmp(&a.size));

                                // Don't show parent entry if we're at root
                                if self
                                    .current_path
                                    .parent()
                                    .map_or(false, |p| p != self.current_path)
                                {
                                    let parent_entry = DirEntry {
                                        path: self.current_path.parent().unwrap().to_path_buf(),
                                        name: "..".to_string(),
                                        size: 0,
                                        is_dir: true,
                                        file_count: 0,
                                        size_change: None,
                                        is_new: false,
                                    };
                                    entries.insert(0, parent_entry);
                                }
                                self.entries = entries;
                                self.selected_index = 0;
                                self.scroll_offset = 0;
                                // Clear any previous error
                                if self
                                    .notification
                                    .as_ref()
                                    .map_or(false, |n| n.contains("✗"))
                                {
                                    self.notification = None;
                                }
                                break; // Exit loop on success
                            }
                            ScanResult::Error(e) => {
                                self.is_scanning = false;
                                self.scan_thread = None;
                                self.scan_rx = None;
                                self.scanning_name = None;
                                self.scan_progress = None;
                                self.entries.clear();
                                self.selected_index = 0;
                                self.scroll_offset = 0;
                                self.notification =
                                    Some(format!("✗ Error reading directory: {}", e));
                                self.notification_time = Some(Instant::now());
                                break; // Exit loop on error
                            }
                        }
                    }
                    Err(_) => break, // No more messages
                }
            }
        }
    }

    pub fn open_delete_modal(&mut self) {
        if let Some(entry) = self.entries.get(self.selected_index) {
            self.modal = Some(Modal::confirm_delete(&entry.path, entry.size));
        }
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn start_delete(&mut self, path: &PathBuf) -> Result<(), String> {
        let path_clone = path.clone();
        let (tx, rx) = mpsc::channel();
        let start_time = Instant::now();

        let handle = thread::spawn(move || {
            match delete::delete_directory(&path_clone) {
                Ok(result) => {
                    let duration_ms = start_time.elapsed().as_millis() as u64;

                    // Log the deletion
                    let log = logger::DeleteLog {
                        timestamp: Local::now().to_rfc3339(),
                        action: "delete".to_string(),
                        path: path_clone.display().to_string(),
                        size_bytes: result.total_bytes,
                        dry_run: false,
                        status: "success".to_string(),
                        files_deleted: result.total_files,
                        duration_ms,
                        errors: if result.errors.is_empty() {
                            None
                        } else {
                            Some(result.errors)
                        },
                    };

                    let _ = logger::write_log(&log);

                    let _ = tx.send(DeleteProgressUpdate::Complete {
                        total_bytes: result.total_bytes,
                        total_files: result.total_files,
                    });
                    Ok(())
                }
                Err(e) => {
                    // Log the error
                    let log = logger::DeleteLog {
                        timestamp: Local::now().to_rfc3339(),
                        action: "delete".to_string(),
                        path: path_clone.display().to_string(),
                        size_bytes: 0,
                        dry_run: false,
                        status: "error".to_string(),
                        files_deleted: 0,
                        duration_ms: start_time.elapsed().as_millis() as u64,
                        errors: Some(vec![e.to_string()]),
                    };

                    let _ = logger::write_log(&log);
                    let _ = tx.send(DeleteProgressUpdate::Error(e.to_string()));
                    Err(e.to_string())
                }
            }
        });

        self.delete_thread = Some(handle);
        self.delete_rx = Some(rx);
        self.mode = AppMode::Deleting;
        self.delete_progress = Some(DeleteProgress {
            deleted_bytes: 0,
            total_bytes: 0, // Will be updated as we scan
            deleted_files: 0,
            total_files: 0,
            current_file: String::new(),
            status: "Starting deletion...".to_string(),
        });

        Ok(())
    }

    pub fn start_dry_run(&mut self, path: &PathBuf) -> Result<(), String> {
        match delete::dry_run_delete(path) {
            Ok(files) => {
                self.mode = AppMode::DryRun;

                // Calculate total size
                let total_size: u64 = files
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum();

                let msg = format!(
                    "Dry-run: Would delete {} files ({:.1} MB)",
                    files.len(),
                    total_size as f64 / 1_000_000.0
                );

                // Log the dry-run
                let log = logger::DeleteLog {
                    timestamp: Local::now().to_rfc3339(),
                    action: "dry-run".to_string(),
                    path: path.display().to_string(),
                    size_bytes: total_size,
                    dry_run: true,
                    status: "complete".to_string(),
                    files_deleted: files.len() as u64,
                    duration_ms: 0,
                    errors: None,
                };

                let _ = logger::write_log(&log);
                self.notification = Some(msg);
                self.notification_time = Some(Instant::now());
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn update_delete_progress(&mut self) {
        let mut updates = Vec::new();
        if let Some(rx) = self.delete_rx.as_mut() {
            while let Ok(update) = rx.try_recv() {
                updates.push(update);
            }
        }

        for update in updates {
            match update {
                DeleteProgressUpdate::Progress {
                    bytes_done,
                    bytes_total,
                    files_done,
                    files_total,
                    current_file,
                } => {
                    if let Some(progress) = &mut self.delete_progress {
                        progress.deleted_bytes = bytes_done;
                        progress.total_bytes = bytes_total;
                        progress.deleted_files = files_done;
                        progress.total_files = files_total;
                        progress.current_file = current_file;
                        progress.status = "Deleting...".to_string();
                    }
                }
                DeleteProgressUpdate::Complete {
                    total_bytes,
                    total_files,
                } => {
                    self.delete_progress = None;
                    self.delete_rx = None;
                    self.mode = AppMode::Browsing;
                    let msg = format!(
                        "✓ Deleted {} files ({:.1} MB)",
                        total_files,
                        total_bytes as f64 / 1_000_000.0
                    );
                    self.notification = Some(msg);
                    self.notification_time = Some(Instant::now());
                    // Clear cache since directory sizes have changed
                    self.size_cache.clear();
                    // Update disk space after deletion
                    self.disk_space = platform::get_disk_space(&self.current_path);
                    self.refresh();
                }
                DeleteProgressUpdate::Error(e) => {
                    self.delete_progress = None;
                    self.delete_rx = None;
                    self.mode = AppMode::Browsing;
                    let msg = format!("✗ Delete error: {}", e);
                    self.notification = Some(msg);
                    self.notification_time = Some(Instant::now());
                }
            }
        }
    }
}

fn apply_size_changes(entries: &mut [DirEntry], changes: &[SizeChange]) {
    if entries.is_empty() || changes.is_empty() {
        return;
    }

    let total_size: u64 = entries.iter().map(|entry| entry.size).sum();

    for entry in entries {
        if let Some(change) = changes.iter().find(|c| c.name == entry.name) {
            let percent_of_directory = if total_size > 0 {
                (change.delta_bytes as f64 / total_size as f64) * 100.0
            } else {
                0.0
            };

            entry.size_change = Some((change.delta_bytes, percent_of_directory as f32));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_entry(name: &str, size: u64) -> DirEntry {
        DirEntry {
            path: PathBuf::from(format!("/{}", name)),
            name: name.to_string(),
            size,
            is_dir: true,
            file_count: 0,
            size_change: None,
            is_new: false,
        }
    }

    #[test]
    fn size_change_percent_matches_directory_ratio() {
        let mut entries = vec![mock_entry("a", 60), mock_entry("b", 40)];
        let changes = vec![SizeChange {
            name: "a".into(),
            old_size: 30,
            new_size: 90,
            delta_bytes: 30,
            delta_percent: 0.0,
        }];

        apply_size_changes(&mut entries, &changes);

        let change = entries[0].size_change.expect("change");
        assert_eq!(change.0, 30);
        assert!((change.1 - 30.0).abs() < f32::EPSILON);
        assert!(entries[1].size_change.is_none());
    }

    #[test]
    fn zero_total_size_yields_zero_percent_change() {
        let mut entries = vec![mock_entry("empty", 0)];
        let changes = vec![SizeChange {
            name: "empty".into(),
            old_size: 0,
            new_size: 100,
            delta_bytes: 100,
            delta_percent: 0.0,
        }];

        apply_size_changes(&mut entries, &changes);

        let change = entries[0].size_change.expect("change");
        assert_eq!(change.0, 100);
        assert_eq!(change.1, 0.0);
    }
}
