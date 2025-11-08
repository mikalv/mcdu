use crate::modal::Modal;
use crate::scan::DirEntry;
use crate::scan;
use crate::delete;
use crate::logger;
use crate::changes::{DirectoryFingerprint, get_fingerprint_path};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use chrono::Local;

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
    pub mode: AppMode,
    pub modal: Option<Modal>,
    pub delete_progress: Option<DeleteProgress>,
    pub delete_thread: Option<JoinHandle<Result<(), String>>>,
    pub delete_rx: Option<mpsc::Receiver<DeleteProgressUpdate>>,
    pub notification: Option<String>,
    pub notification_time: Option<Instant>,
    pub show_help: bool,
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
        let root = PathBuf::from(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")));
        let mut app = App {
            _root_path: root.clone(),
            current_path: root.clone(),
            entries: Vec::new(),
            selected_index: 0,
            mode: AppMode::Browsing,
            modal: None,
            delete_progress: None,
            delete_thread: None,
            delete_rx: None,
            notification: None,
            notification_time: None,
            show_help: false,
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

    pub fn enter_directory(&mut self) {
        if let Some(entry) = self.entries.get(self.selected_index) {
            if entry.is_dir {
                self.current_path = entry.path.clone();
                self.selected_index = 0;
                self.refresh();
            }
        }
    }

    pub fn go_parent(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            if parent != self.current_path {
                self.current_path = parent.to_path_buf();
                self.selected_index = 0;
                self.refresh();
            }
        }
    }

    pub fn refresh(&mut self) {
        match scan::scan_directory(&self.current_path) {
            Ok(mut entries) => {
                // Load previous fingerprint and detect changes
                let fp_path = get_fingerprint_path(&self.current_path);

                // Create fingerprint from scanned entries (without re-reading metadata)
                let new_fp = DirectoryFingerprint::from_entries(&entries);

                if let Ok(old_fp) = DirectoryFingerprint::load(&fp_path) {
                    let changes = old_fp.get_changes(&new_fp);

                    // Apply changes to entries
                    for entry in &mut entries {
                        if let Some(change) = changes.iter().find(|c| c.name == entry.name) {
                            entry.size_change = Some((change.delta_bytes, change.delta_percent));
                        }
                    }
                }

                // Save new fingerprint for next run
                let _ = std::fs::create_dir_all(fp_path.parent().unwrap());
                let _ = new_fp.save(&fp_path);

                // Don't show parent entry if we're at root
                if self.current_path.parent().map_or(false, |p| p != self.current_path) {
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
                // Clear any previous error
                if self.notification.as_ref().map_or(false, |n| n.contains("✗")) {
                    self.notification = None;
                }
            }
            Err(e) => {
                self.entries.clear();
                self.notification = Some(format!("✗ Error reading directory: {}", e));
                self.notification_time = Some(Instant::now());
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
                        errors: if result.errors.is_empty() { None } else { Some(result.errors) },
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
                let total_size: u64 = files.iter()
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
