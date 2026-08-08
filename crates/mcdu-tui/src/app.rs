use crate::cleanup_ui::CleanupTab;
use crate::delete;
use crate::logger;
use crate::modal::Modal;
use crate::tree::{scan_tree_cancellable, FileNode, ScanProgress};
use chrono::Local;
use mcdu_core as cleanup;
use mcdu_core::platform::{self, DiskSpace};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Browsing,
    Deleting,
    DryRun,
    Cleanup,
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
    pub root_path: PathBuf,
    pub tree: Option<FileNode>,
    pub nav_stack: Vec<usize>, // Indices into children at each level
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub mode: AppMode,
    pub modal: Option<Modal>,
    pub delete_progress: Option<DeleteProgress>,
    pub delete_thread: Option<JoinHandle<Result<(), String>>>,
    pub delete_rx: Option<mpsc::Receiver<DeleteProgressUpdate>>,
    pub deleting_path: Option<PathBuf>, // Path being deleted (for tree update)
    pub notification: Option<String>,
    pub notification_time: Option<Instant>,
    pub show_help: bool,
    // Async scanning
    pub scan_thread: Option<JoinHandle<()>>,
    pub scan_rx: Option<mpsc::Receiver<ScanProgress>>,
    pub scan_cancel: Option<Arc<AtomicBool>>,
    pub is_scanning: bool,
    pub scan_files_count: usize,
    pub scanning_path: Option<String>,
    /// When set, incremental scan events apply to this subtree (path-based).
    pub rescan_target: Option<PathBuf>,
    // Disk space info
    pub disk_space: Option<DiskSpace>,
    // Cleanup feature
    pub cleanup_candidates: Vec<mcdu_core::rules::Candidate>,
    pub cleanup_selected: std::collections::HashSet<std::path::PathBuf>,
    pub cleanup_selected_index: usize,
    pub cleanup_categories: Vec<mcdu_core::scanner::CategoryGroup>,
    pub cleanup_expanded: std::collections::HashSet<String>,
    pub cleanup_scan_thread: Option<std::thread::JoinHandle<Vec<mcdu_core::rules::Candidate>>>,
    pub cleanup_scan_rx: Option<std::sync::mpsc::Receiver<mcdu_core::scanner::ScanProgress>>,
    pub cleanup_scanning: bool,
    pub cleanup_scan_progress: Option<mcdu_core::scanner::ScanProgress>,
    pub cleanup_delete_thread: Option<std::thread::JoinHandle<mcdu_core::executor::CleanupResult>>,
    pub cleanup_delete_rx: Option<mpsc::Receiver<mcdu_core::executor::CleanupProgress>>,
    pub cleanup_delete_progress: Option<mcdu_core::executor::CleanupProgress>,
    pub cleanup_pending: Option<(Vec<mcdu_core::rules::Candidate>, bool)>,
    pub cleanup_active_tab: crate::cleanup_ui::CleanupTab,
    pub cleanup_files_sort: crate::cleanup_ui::FilesSortColumn,
    pub cleanup_files_sort_desc: bool,
    pub cleanup_files_scroll: usize,
    pub cleanup_files_selected: usize,
    pub cleanup_quarantine_list: Vec<mcdu_core::QuarantineManifest>,
    pub cleanup_quarantine_selected: usize,
    // Splash screen state (only with "splash" feature)
    #[cfg(feature = "splash")]
    pub splash_state: Option<crate::splash::SplashState>,
}

pub enum DeleteProgressUpdate {
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

/// Entry for display in the UI (derived from tree)
pub struct DisplayEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub complete: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        Self::new_with_root(root)
    }

    pub fn new_with_root(root: PathBuf) -> Self {
        let mut app = Self::create(root, true);
        app.start_scan();
        app.refresh_quarantine_list();
        app
    }

    /// Cleanup/orphans entry: skip the browser tree scan and splash wait.
    pub fn new_cleanup_mode() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut app = Self::create(root, false);
        app.mode = AppMode::Cleanup;
        #[cfg(feature = "splash")]
        {
            app.splash_state = None;
        }
        app.is_scanning = false;
        app.refresh_quarantine_list();
        app
    }

    fn create(root: PathBuf, with_splash: bool) -> Self {
        let disk_space = platform::get_disk_space(&root);

        App {
            root_path: root,
            tree: None,
            nav_stack: Vec::new(),
            selected_index: 0,
            scroll_offset: 0,
            mode: AppMode::Browsing,
            modal: None,
            delete_progress: None,
            delete_thread: None,
            delete_rx: None,
            deleting_path: None,
            notification: None,
            notification_time: None,
            show_help: false,
            scan_thread: None,
            scan_rx: None,
            scan_cancel: None,
            is_scanning: false,
            scan_files_count: 0,
            scanning_path: None,
            rescan_target: None,
            disk_space,
            cleanup_candidates: Vec::new(),
            cleanup_selected: HashSet::new(),
            cleanup_selected_index: 0,
            cleanup_categories: Vec::new(),
            cleanup_expanded: HashSet::new(),
            cleanup_scan_thread: None,
            cleanup_scan_rx: None,
            cleanup_scanning: false,
            cleanup_scan_progress: None,
            cleanup_delete_thread: None,
            cleanup_delete_rx: None,
            cleanup_delete_progress: None,
            cleanup_pending: None,
            cleanup_active_tab: crate::cleanup_ui::CleanupTab::Overview,
            cleanup_files_sort: crate::cleanup_ui::FilesSortColumn::Size,
            cleanup_files_sort_desc: true,
            cleanup_files_scroll: 0,
            cleanup_files_selected: 0,
            cleanup_quarantine_list: Vec::new(),
            cleanup_quarantine_selected: 0,
            #[cfg(feature = "splash")]
            splash_state: if with_splash {
                Some(crate::splash::SplashState::new())
            } else {
                None
            },
        }
    }

    /// Get the current directory node we're viewing
    pub fn get_current_node(&self) -> Option<&FileNode> {
        let tree = self.tree.as_ref()?;
        let mut node = tree;

        for &idx in &self.nav_stack {
            node = node.children.get(idx)?;
        }

        Some(node)
    }

    /// Get entries for display (current directory's children)
    pub fn get_display_entries(&self) -> Vec<DisplayEntry> {
        let mut entries = Vec::new();

        // Add parent entry if not at root
        if !self.nav_stack.is_empty() {
            entries.push(DisplayEntry {
                name: "..".to_string(),
                path: PathBuf::new(),
                size: 0,
                is_dir: true,
                complete: true,
            });
        }

        if let Some(node) = self.get_current_node() {
            for child in &node.children {
                entries.push(DisplayEntry {
                    name: child.name.clone(),
                    path: child.path.clone(),
                    size: child.size,
                    is_dir: child.is_dir,
                    complete: child.complete,
                });
            }
        }

        entries
    }

    /// Get current path for display
    pub fn get_current_path(&self) -> PathBuf {
        self.get_current_node()
            .map(|n| n.path.clone())
            .unwrap_or_else(|| self.root_path.clone())
    }

    /// Get total entries count
    pub fn entries_count(&self) -> usize {
        let base = self
            .get_current_node()
            .map(|n| n.children.len())
            .unwrap_or(0);
        if self.nav_stack.is_empty() {
            base
        } else {
            base + 1 // +1 for ".." entry
        }
    }

    pub fn select_next(&mut self) {
        let count = self.entries_count();
        if count > 0 && self.selected_index < count - 1 {
            self.selected_index += 1;
        }
    }

    pub fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn adjust_scroll(&mut self, viewport_height: usize) {
        let usable_height = viewport_height.saturating_sub(2);

        if usable_height == 0 {
            return;
        }

        if self.selected_index < self.scroll_offset {
            self.scroll_offset = self.selected_index;
        }

        if self.selected_index >= self.scroll_offset + usable_height {
            self.scroll_offset = self.selected_index.saturating_sub(usable_height - 1);
        }
    }

    pub fn enter_directory(&mut self) {
        if self.tree.is_none() {
            return;
        }

        let entries = self.get_display_entries();
        if let Some(entry) = entries.get(self.selected_index) {
            if !entry.is_dir {
                return;
            }

            // Handle ".." entry
            if entry.name == ".." {
                self.go_parent();
                return;
            }

            // Find the index of this child in the current node
            if let Some(current) = self.get_current_node() {
                let child_idx = current.children.iter().position(|c| c.name == entry.name);
                if let Some(idx) = child_idx {
                    self.nav_stack.push(idx);
                    self.selected_index = 0;
                    self.scroll_offset = 0;
                }
            }
        }
    }

    pub fn go_parent(&mut self) {
        if self.nav_stack.pop().is_some() {
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    fn nav_path_stack(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let Some(tree) = self.tree.as_ref() else {
            return paths;
        };
        let mut node = tree;
        for &idx in &self.nav_stack {
            if idx >= node.children.len() {
                break;
            }
            paths.push(node.children[idx].path.clone());
            node = &node.children[idx];
        }
        paths
    }

    fn selected_entry_path(&self) -> Option<PathBuf> {
        let entries = self.get_display_entries();
        entries.get(self.selected_index).and_then(|e| {
            if e.name == ".." {
                None
            } else {
                Some(e.path.clone())
            }
        })
    }

    fn remap_navigation(&mut self, nav_paths: &[PathBuf], selected: Option<PathBuf>) {
        let Some(tree) = self.tree.as_ref() else {
            return;
        };

        let mut remapped = Vec::new();
        let mut cursor = tree;
        for path in nav_paths {
            if let Some(idx) = cursor.children.iter().position(|c| &c.path == path) {
                remapped.push(idx);
                cursor = &cursor.children[idx];
            } else {
                break;
            }
        }
        self.nav_stack = remapped;

        let entries = self.get_display_entries();
        if let Some(sel) = selected {
            if let Some(idx) = entries.iter().position(|e| e.path == sel) {
                self.selected_index = idx;
            } else if !entries.is_empty() {
                self.selected_index = self.selected_index.min(entries.len() - 1);
            } else {
                self.selected_index = 0;
            }
        } else if !entries.is_empty() {
            self.selected_index = self.selected_index.min(entries.len() - 1);
        }

        let count = entries.len();
        if count > 0 && self.scroll_offset >= count {
            self.scroll_offset = count.saturating_sub(1);
        }
    }

    fn apply_listed(&mut self, path: PathBuf, children: Vec<FileNode>) {
        let selected = self.selected_entry_path();
        let nav_paths = self.nav_path_stack();

        if self.tree.is_none() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            let mut root = FileNode::new_dir(name, path);
            root.children = children;
            root.complete = false;
            root.size = root.children.iter().map(|c| c.size).sum();
            root.sort_children_by_size();
            self.tree = Some(root);

            #[cfg(feature = "splash")]
            if let Some(ref mut splash) = self.splash_state {
                splash.start_fadeout();
            }
            return;
        }

        if let Some(node) = find_node_mut(self.tree.as_mut().unwrap(), &path) {
            node.children = children;
            node.complete = false;
            node.size = node.children.iter().map(|c| c.size).sum();
            node.sort_children_by_size();
        }

        self.remap_navigation(&nav_paths, selected);
    }

    fn apply_sized(&mut self, path: PathBuf, size: u64, complete: bool) {
        let selected = self.selected_entry_path();
        let nav_paths = self.nav_path_stack();

        let diff = {
            let Some(tree) = self.tree.as_mut() else {
                return;
            };
            let Some(node) = find_node_mut(tree, &path) else {
                return;
            };
            let old_size = node.size;
            node.size = size;
            node.complete = complete;
            size as i64 - old_size as i64
        };

        if diff != 0 {
            if let Some(tree) = self.tree.as_mut() {
                apply_delta_to_ancestors(tree, &path, diff);
            }
        }
        if let Some(tree) = self.tree.as_mut() {
            resort_along_path(tree, &path);
        }

        self.remap_navigation(&nav_paths, selected);
    }

    /// Remove a deleted entry from the tree and update sizes up the tree
    fn remove_entry_from_tree(&mut self, path: &std::path::Path) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };

        // Navigate to the current node using nav_stack
        let mut node = tree;
        for &idx in &self.nav_stack {
            node = &mut node.children[idx];
        }

        // Find and remove the child with matching path
        if let Some(idx) = node.children.iter().position(|c| c.path == path) {
            let removed_size = node.children[idx].size;
            node.children.remove(idx);

            // Update selected_index if needed
            if self.selected_index >= node.children.len() && self.selected_index > 0 {
                // Account for ".." entry if present
                let offset = if self.nav_stack.is_empty() { 0 } else { 1 };
                let max_idx = node.children.len().saturating_sub(1) + offset;
                self.selected_index = max_idx;
            }

            // Subtract the removed size from all parents up to root
            if removed_size > 0 {
                let tree = self.tree.as_mut().unwrap();
                tree.size = tree.size.saturating_sub(removed_size);

                let mut parent = tree;
                for &idx in &self.nav_stack {
                    parent = &mut parent.children[idx];
                    parent.size = parent.size.saturating_sub(removed_size);
                }
            }
        }
    }

    /// Start full tree scan
    fn start_scan(&mut self) {
        // Signal previous scan to stop; do not block the UI thread on join
        if let Some(cancel) = self.scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.scan_thread = None;
        self.scan_rx = None;

        let path = self.root_path.clone();
        // Bounded channel: scanner blocks when UI is behind instead of freezing the UI
        let (tx, rx) = mpsc::sync_channel(128);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let handle = thread::spawn(move || {
            match scan_tree_cancellable(&path, Some(tx.clone()), Some(cancel_clone)) {
                Ok(_) => {
                    // Complete already sent by scanner
                }
                Err(e) => {
                    if e != "scan cancelled" {
                        let _ = tx.send(ScanProgress::Error(e));
                    }
                }
            }
        });

        self.scan_cancel = Some(cancel);
        self.scan_thread = Some(handle);
        self.scan_rx = Some(rx);
        self.is_scanning = true;
        self.scan_files_count = 0;
        self.scanning_path = None;
    }

    pub fn refresh(&mut self) {
        self.tree = None;
        self.nav_stack.clear();
        self.selected_index = 0;
        self.scroll_offset = 0;
        self.rescan_target = None;
        self.start_scan();
    }

    /// Rescan just the selected directory (subtree only)
    pub fn rescan_selected(&mut self) {
        if self.is_scanning || self.tree.is_none() {
            return;
        }

        let entries = self.get_display_entries();
        let Some(entry) = entries.get(self.selected_index) else {
            return;
        };

        if !entry.is_dir || entry.name == ".." {
            self.notification = Some("Select a directory to rescan".to_string());
            self.notification_time = Some(Instant::now());
            return;
        }

        let path = entry.path.clone();

        // Clear target subtree so UI shows incomplete state while rescanning
        let old_size = {
            let tree = self.tree.as_mut().unwrap();
            if let Some(node) = find_node_mut(tree, &path) {
                let old = node.size;
                node.children.clear();
                node.size = 0;
                node.complete = false;
                Some(old)
            } else {
                None
            }
        };
        if let Some(old_size) = old_size {
            if old_size > 0 {
                if let Some(tree) = self.tree.as_mut() {
                    apply_delta_to_ancestors(tree, &path, -(old_size as i64));
                }
            }
        }

        self.rescan_target = Some(path.clone());

        if let Some(cancel) = self.scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let (tx, rx) = mpsc::sync_channel(128);
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let handle = thread::spawn(move || {
            match scan_tree_cancellable(&path, Some(tx.clone()), Some(cancel_clone)) {
                Ok(_) => {}
                Err(e) => {
                    if e != "scan cancelled" {
                        let _ = tx.send(ScanProgress::Error(e));
                    }
                }
            }
        });

        self.scan_cancel = Some(cancel);
        self.scan_thread = Some(handle);
        self.scan_rx = Some(rx);
        self.is_scanning = true;
        self.scan_files_count = 0;
        self.scanning_path = None;
    }

    /// Drain a bounded amount of scan progress so the UI stays interactive.
    /// Returns true if state changed and a redraw is needed.
    pub fn update_scan_progress(&mut self) -> bool {
        // Take receiver to avoid holding borrow across &mut self helpers
        let Some(rx) = self.scan_rx.take() else {
            return false;
        };

        let mut keep_rx = true;
        let mut changed = false;
        let started = Instant::now();
        let mut processed = 0usize;
        const MAX_EVENTS: usize = 32;
        const MAX_MS: u128 = 6;

        while processed < MAX_EVENTS && started.elapsed().as_millis() < MAX_MS {
            match rx.try_recv() {
                Ok(progress) => {
                    processed += 1;
                    match progress {
                        ScanProgress::Scanning {
                            files_scanned,
                            current_path,
                        } => {
                            self.scan_files_count = files_scanned;
                            self.scanning_path = Some(current_path);
                            changed = true;
                        }
                        ScanProgress::Listed { path, children } => {
                            self.apply_listed(path, children);
                            changed = true;
                        }
                        ScanProgress::Sized {
                            path,
                            size,
                            complete,
                        } => {
                            self.apply_sized(path, size, complete);
                            changed = true;
                        }
                        ScanProgress::Complete => {
                            self.is_scanning = false;
                            self.scan_thread = None;
                            self.scanning_path = None;
                            keep_rx = false;
                            changed = true;

                            if self.rescan_target.take().is_some() {
                                self.notification = Some("✓ Subtree rescanned".to_string());
                                self.notification_time = Some(Instant::now());
                            }

                            self.disk_space = platform::get_disk_space(&self.root_path);
                            break;
                        }
                        ScanProgress::Error(e) => {
                            self.is_scanning = false;
                            self.scan_thread = None;
                            self.scanning_path = None;
                            self.rescan_target = None;
                            keep_rx = false;
                            changed = true;
                            self.notification = Some(format!("✗ Scan error: {}", e));
                            self.notification_time = Some(Instant::now());
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }

        if keep_rx {
            self.scan_rx = Some(rx);
        }
        changed
    }

    pub fn open_delete_modal(&mut self) {
        let entries = self.get_display_entries();
        if let Some(entry) = entries.get(self.selected_index) {
            if entry.name != ".." {
                self.modal = Some(Modal::confirm_delete(&entry.path, entry.size));
            }
        }
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn start_cleanup_scan(&mut self) -> Result<(), String> {
        self.start_cleanup_scan_with_path(None)
    }

    pub fn start_cleanup_scan_with_path(
        &mut self,
        scan_path: Option<PathBuf>,
    ) -> Result<(), String> {
        let platform_paths = cleanup::platform::PlatformPaths::detect()
            .ok_or_else(|| "Unable to detect platform paths".to_string())?;
        let config_paths = cleanup::config::default_config_paths(&platform_paths);
        let mut config = cleanup::config::load_config(&config_paths).map_err(|e| e.to_string())?;

        if let Some(path) = scan_path {
            config.scan_paths = vec![path.to_string_lossy().to_string()];
        }
        let state = cleanup::config::load_state(&config_paths).map_err(|e| e.to_string())?;
        self.cleanup_selected = state.selected.into_iter().map(PathBuf::from).collect();
        self.cleanup_selected_index = 0;

        let (tx, rx) = mpsc::channel();
        let config_clone = config.clone();
        let platform_clone = platform_paths.clone();
        let handle = thread::spawn(move || {
            let mut results = cleanup::scanner::scan(
                &config_clone,
                &platform_clone,
                Some(tx),
                std::time::SystemTime::now(),
            );

            // Append orphaned app data on macOS
            #[cfg(target_os = "macos")]
            {
                match mcdu_macos::scan_orphans(&platform_clone.home_dir, None) {
                    Ok(orphans) => results.extend(orphans),
                    Err(_) => {
                        // Discovery failure: skip orphans rather than flag everything
                    }
                }
            }

            results
        });

        self.cleanup_scan_thread = Some(handle);
        self.cleanup_scan_rx = Some(rx);
        self.cleanup_scanning = true;
        self.mode = AppMode::Cleanup;
        Ok(())
    }

    /// Start a scan that only finds orphaned macOS app data (no rule-based scan)
    #[cfg(target_os = "macos")]
    pub fn start_orphan_scan(&mut self) -> Result<(), String> {
        let platform_paths = cleanup::platform::PlatformPaths::detect()
            .ok_or_else(|| "Unable to detect platform paths".to_string())?;

        self.cleanup_selected = HashSet::new();
        self.cleanup_selected_index = 0;

        let (tx, rx) = mpsc::channel();
        let home = platform_paths.home_dir.clone();
        let handle = thread::spawn(move || {
            mcdu_macos::scan_orphans(&home, Some(&tx)).unwrap_or_default()
        });

        self.cleanup_scan_thread = Some(handle);
        self.cleanup_scan_rx = Some(rx);
        self.cleanup_scanning = true;
        self.mode = AppMode::Cleanup;
        Ok(())
    }

    pub fn update_cleanup_scan(&mut self) {
        if let Some(rx) = self.cleanup_scan_rx.as_mut() {
            while let Ok(progress) = rx.try_recv() {
                self.cleanup_scan_progress = Some(progress);
            }
        }

        if let Some(handle) = self.cleanup_scan_thread.as_ref() {
            if handle.is_finished() {
                let handle = self.cleanup_scan_thread.take().unwrap();
                self.cleanup_scanning = false;
                self.cleanup_scan_progress = None;
                self.cleanup_scan_rx = None;

                if let Ok(results) = handle.join() {
                    self.cleanup_candidates = results;
                    self.cleanup_categories =
                        cleanup::scanner::group_by_category(self.cleanup_candidates.clone());
                    self.cleanup_expanded = self
                        .cleanup_categories
                        .iter()
                        .map(|c| c.name.clone())
                        .collect();
                    self.apply_selection_and_save();
                    self.notification = Some(format!(
                        "Cleanup scan complete: {} candidates",
                        self.cleanup_candidates.len()
                    ));
                } else {
                    self.notification = Some("Cleanup scan failed".to_string());
                }
                self.notification_time = Some(Instant::now());
            }
        }
    }

    #[allow(dead_code)]
    pub fn block_on_cleanup_scan(&mut self) {
        while self.cleanup_scanning {
            self.update_cleanup_scan();
            if self.cleanup_scanning {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn apply_selection_and_save(&mut self) {
        // Retain only candidates that still exist
        let candidate_paths: HashSet<PathBuf> = self
            .cleanup_candidates
            .iter()
            .map(|c| c.path.clone())
            .collect();
        self.cleanup_selected
            .retain(|p| candidate_paths.contains(p));
        if self.cleanup_selected.is_empty() {
            // Only auto-select candidates that opt in (e.g., orphans are unchecked by default)
            self.cleanup_selected = self
                .cleanup_candidates
                .iter()
                .filter(|c| c.default_selected)
                .map(|c| c.path.clone())
                .collect();
        }

        let platform_paths = match cleanup::platform::PlatformPaths::detect() {
            Some(p) => p,
            None => return,
        };
        let config_paths = cleanup::config::default_config_paths(&platform_paths);
        let state = cleanup::config::CleanupState {
            selected: self
                .cleanup_selected
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            dismissed: Vec::new(),
        };
        let _ = cleanup::config::save_state(&config_paths, &state);
    }

    pub fn cleanup_rows(&self) -> Vec<CleanupRow> {
        let mut rows = Vec::new();
        for cat in &self.cleanup_categories {
            rows.push(CleanupRow::Category {
                name: cat.name.clone(),
            });
            if self.cleanup_expanded.contains(&cat.name) {
                for cand in &cat.candidates {
                    rows.push(CleanupRow::Candidate {
                        path: cand.path.clone(),
                        rule: cand.rule_name.clone(),
                        pattern: cand.rule_pattern.clone(),
                        size: cand.size_bytes,
                    });
                }
            }
        }
        rows
    }

    pub fn select_next_cleanup(&mut self) {
        let rows = self.cleanup_rows();
        if self.cleanup_selected_index + 1 < rows.len() {
            self.cleanup_selected_index += 1;
        }
    }

    pub fn select_previous_cleanup(&mut self) {
        if self.cleanup_selected_index > 0 {
            self.cleanup_selected_index -= 1;
        }
    }

    pub fn next_cleanup_tab(&mut self) {
        self.cleanup_active_tab = match self.cleanup_active_tab {
            CleanupTab::Overview => CleanupTab::Categories,
            CleanupTab::Categories => CleanupTab::Files,
            CleanupTab::Files => CleanupTab::Quarantine,
            CleanupTab::Quarantine => CleanupTab::Overview,
        };
        if matches!(self.cleanup_active_tab, CleanupTab::Quarantine) {
            self.refresh_quarantine_list();
        }
    }

    pub fn prev_cleanup_tab(&mut self) {
        self.cleanup_active_tab = match self.cleanup_active_tab {
            CleanupTab::Overview => CleanupTab::Quarantine,
            CleanupTab::Categories => CleanupTab::Overview,
            CleanupTab::Files => CleanupTab::Categories,
            CleanupTab::Quarantine => CleanupTab::Files,
        };
        if matches!(self.cleanup_active_tab, CleanupTab::Quarantine) {
            self.refresh_quarantine_list();
        }
    }

    pub fn set_cleanup_tab(&mut self, tab: CleanupTab) {
        self.cleanup_active_tab = tab;
        if matches!(tab, CleanupTab::Quarantine) {
            self.refresh_quarantine_list();
        }
    }

    pub fn refresh_quarantine_list(&mut self) {
        let q = cleanup::default_quarantine();
        self.cleanup_quarantine_list = q.list().unwrap_or_default();
        if self.cleanup_quarantine_selected >= self.cleanup_quarantine_list.len()
            && !self.cleanup_quarantine_list.is_empty()
        {
            self.cleanup_quarantine_selected = self.cleanup_quarantine_list.len() - 1;
        }
        if self.cleanup_quarantine_list.is_empty() {
            self.cleanup_quarantine_selected = 0;
        }
    }

    pub fn restore_quarantine_selected(&mut self) {
        let Some(manifest) = self
            .cleanup_quarantine_list
            .get(self.cleanup_quarantine_selected)
            .cloned()
        else {
            self.notification = Some("No quarantine batch selected".into());
            self.notification_time = Some(Instant::now());
            return;
        };
        let q = cleanup::default_quarantine();
        match q.restore(&manifest.id) {
            Ok(paths) => {
                self.notification = Some(format!("Restored {} item(s)", paths.len()));
                self.notification_time = Some(Instant::now());
            }
            Err(e) => {
                self.notification = Some(format!("Restore failed: {e}"));
                self.notification_time = Some(Instant::now());
            }
        }
        self.refresh_quarantine_list();
    }

    pub fn purge_quarantine_selected(&mut self) {
        let Some(manifest) = self
            .cleanup_quarantine_list
            .get(self.cleanup_quarantine_selected)
            .cloned()
        else {
            self.notification = Some("No quarantine batch selected".into());
            self.notification_time = Some(Instant::now());
            return;
        };
        let q = cleanup::default_quarantine();
        match q.purge(&manifest.id) {
            Ok(bytes) => {
                self.notification = Some(format!("Purged batch ({bytes} bytes)"));
                self.notification_time = Some(Instant::now());
            }
            Err(e) => {
                self.notification = Some(format!("Purge failed: {e}"));
                self.notification_time = Some(Instant::now());
            }
        }
        self.refresh_quarantine_list();
    }

    pub fn toggle_files_selection(&mut self) {
        let mut paths: Vec<_> = self.cleanup_candidates.iter().map(|c| c.path.clone()).collect();
        // Keep sorted view aligned with UI — use same order as draw_cleanup_files would
        paths.sort();
        // Prefer index into cleanup_candidates after applying current sort in UI;
        // use sorted candidate list by size/name matching files tab.
        let sorted = self.sorted_cleanup_candidates();
        if let Some(c) = sorted.get(self.cleanup_files_selected) {
            let path = c.path.clone();
            if self.cleanup_selected.contains(&path) {
                self.cleanup_selected.remove(&path);
            } else {
                self.cleanup_selected.insert(path);
            }
            self.apply_selection_and_save();
        }
    }

    pub fn cycle_files_sort(&mut self) {
        if self.cleanup_files_sort_desc
            && matches!(
                self.cleanup_files_sort,
                crate::cleanup_ui::FilesSortColumn::Size
            )
        {
            // First press on Size: flip direction; subsequent cycle column
            self.cleanup_files_sort_desc = false;
        } else {
            self.cleanup_files_sort = self.cleanup_files_sort.next();
            self.cleanup_files_sort_desc = true;
        }
        self.cleanup_files_selected = 0;
        self.cleanup_files_scroll = 0;
    }

    fn sorted_cleanup_candidates(&self) -> Vec<&cleanup::rules::Candidate> {
        let mut all: Vec<_> = self.cleanup_candidates.iter().collect();
        match self.cleanup_files_sort {
            crate::cleanup_ui::FilesSortColumn::Size => {
                all.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
            }
            crate::cleanup_ui::FilesSortColumn::Name => {
                all.sort_by(|a, b| a.path.cmp(&b.path));
            }
            crate::cleanup_ui::FilesSortColumn::Category => {
                all.sort_by(|a, b| a.rule_category.cmp(&b.rule_category));
            }
            crate::cleanup_ui::FilesSortColumn::Age => {
                all.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));
            }
        }
        if !self.cleanup_files_sort_desc
            && matches!(
                self.cleanup_files_sort,
                crate::cleanup_ui::FilesSortColumn::Size
            )
        {
            all.reverse();
        }
        all
    }

    pub fn toggle_cleanup_selection(&mut self) {
        match self.cleanup_rows().get(self.cleanup_selected_index) {
            Some(CleanupRow::Category { name }) => {
                let paths: Vec<_> = self
                    .cleanup_categories
                    .iter()
                    .find(|c| &c.name == name)
                    .map(|c| c.candidates.iter().map(|cand| cand.path.clone()).collect())
                    .unwrap_or_else(Vec::new);
                let all_selected = paths.iter().all(|p| self.cleanup_selected.contains(p));
                if all_selected {
                    for p in paths {
                        self.cleanup_selected.remove(&p);
                    }
                } else {
                    for p in paths {
                        self.cleanup_selected.insert(p);
                    }
                }
                self.apply_selection_and_save();
            }
            Some(CleanupRow::Candidate { path, .. }) => {
                if self.cleanup_selected.contains(path) {
                    self.cleanup_selected.remove(path);
                } else {
                    self.cleanup_selected.insert(path.clone());
                }
                self.apply_selection_and_save();
            }
            None => {}
        }
    }

    pub fn toggle_cleanup_expand(&mut self) {
        if let Some(CleanupRow::Category { name }) =
            self.cleanup_rows().get(self.cleanup_selected_index)
        {
            if self.cleanup_expanded.contains(name) {
                self.cleanup_expanded.remove(name);
            } else {
                self.cleanup_expanded.insert(name.clone());
            }
        }
    }

    pub fn select_all_cleanup(&mut self) {
        self.cleanup_selected = self
            .cleanup_candidates
            .iter()
            .map(|c| c.path.clone())
            .collect();
        self.apply_selection_and_save();
    }

    pub fn select_none_cleanup(&mut self) {
        self.cleanup_selected.clear();
        self.apply_selection_and_save();
    }

    pub fn start_cleanup_delete(&mut self) {
        let selection = self.cleanup_selection();
        if selection.is_empty() {
            self.notification = Some("No cleanup items selected".to_string());
            self.notification_time = Some(Instant::now());
            return;
        }
        let total_size: u64 = selection.iter().map(|c| c.size_bytes).sum();
        self.cleanup_pending = Some((selection, false));
        self.modal = Some(Modal::cleanup_confirm(
            self.cleanup_pending.as_ref().unwrap().0.len(),
            total_size,
            false,
        ));
    }

    pub fn update_cleanup_delete(&mut self) {
        if let Some(rx) = self.cleanup_delete_rx.as_mut() {
            while let Ok(progress) = rx.try_recv() {
                self.cleanup_delete_progress = Some(progress);
            }
        }

        if let Some(handle) = self.cleanup_delete_thread.as_ref() {
            if handle.is_finished() {
                let handle = self.cleanup_delete_thread.take().unwrap();
                match handle.join() {
                    Ok(result) => {
                        self.cleanup_delete_progress = None;
                        self.cleanup_delete_rx = None;
                        if result.errors.is_empty() {
                            self.notification = Some(format!(
                                "Cleanup deleted files, freed {} bytes",
                                result.freed_bytes
                            ));
                        } else {
                            let detail: Vec<String> = result
                                .errors
                                .iter()
                                .take(3)
                                .map(|(p, e)| format!("{}: {}", p.display(), e))
                                .collect();
                            self.notification = Some(format!(
                                "Cleanup completed with {} errors: {}",
                                result.errors.len(),
                                detail.join("; ")
                            ));
                        }
                        self.notification_time = Some(Instant::now());

                        let deleted: HashSet<_> = result.deleted_paths.into_iter().collect();
                        self.cleanup_candidates
                            .retain(|c| !deleted.contains(&c.path));
                        self.cleanup_selected.retain(|p| !deleted.contains(p));
                        self.cleanup_categories =
                            mcdu_core::group_by_category(self.cleanup_candidates.clone());
                        self.refresh_quarantine_list();

                        let max_idx = self.cleanup_rows().len().saturating_sub(1);
                        if self.cleanup_selected_index > max_idx {
                            self.cleanup_selected_index = max_idx;
                        }

                        self.mode = AppMode::Cleanup;
                    }
                    Err(_) => {
                        self.cleanup_delete_progress = None;
                        self.cleanup_delete_rx = None;
                        self.notification =
                            Some("Cleanup delete thread panicked".to_string());
                        self.notification_time = Some(Instant::now());
                        self.mode = AppMode::Cleanup;
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn block_on_cleanup_delete(&mut self) {
        loop {
            self.update_cleanup_delete();
            if self.cleanup_delete_thread.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn handle_cleanup_modal_confirm(&mut self, action: bool) {
        if !action {
            self.cleanup_pending = None;
            return;
        }
        if let Some((pending, dry_run)) = self.cleanup_pending.take() {
            if dry_run {
                let result = cleanup::executor::dry_run(pending.clone());
                self.cleanup_delete_progress = Some(cleanup::executor::CleanupProgress {
                    path: PathBuf::new(),
                    current: pending.len() as u64,
                    total: pending.len() as u64,
                    freed_bytes: result.freed_bytes,
                    stage: cleanup::executor::CleanupStage::Files,
                });
                self.notification = Some(format!(
                    "Dry-run: {} bytes would be freed",
                    result.freed_bytes
                ));
                self.notification_time = Some(Instant::now());
                self.mode = AppMode::Cleanup;
            } else {
                let total_size: u64 = pending.iter().map(|c| c.size_bytes).sum();
                let has_risky = pending.iter().any(|c| c.risky);
                self.cleanup_pending = Some((pending, false));
                self.modal = Some(Modal::cleanup_final(
                    self.cleanup_pending.as_ref().unwrap().0.len(),
                    total_size,
                    has_risky,
                ));
            }
        }
    }

    pub fn handle_cleanup_final_confirm(&mut self, action: bool) {
        self.handle_cleanup_final_confirm_with_git(action, false);
    }

    pub fn handle_cleanup_final_confirm_with_git(&mut self, action: bool, run_git: bool) {
        if !action {
            self.cleanup_pending = None;
            return;
        }
        if let Some((pending, _)) = self.cleanup_pending.take() {
            let git_roots = if run_git {
                self.cleanup_git_roots(&pending)
            } else {
                Vec::new()
            };
            let (tx, rx) = mpsc::channel();
            let handle = cleanup::executor::execute_async(pending, run_git, git_roots, Some(tx));
            self.cleanup_delete_thread = Some(handle);
            self.cleanup_delete_rx = Some(rx);
            self.notification = Some("Starting cleanup delete...".to_string());
            self.notification_time = Some(Instant::now());
        }
    }

    fn cleanup_git_roots(&self, candidates: &[cleanup::rules::Candidate]) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        for cand in candidates {
            if let Some(parent) = cand.path.parent() {
                roots.push(parent.to_path_buf());
            }
        }
        roots.sort();
        roots.dedup();
        roots
    }

    pub fn start_cleanup_dry_run(&mut self) {
        let selection = self.cleanup_selection();
        if selection.is_empty() {
            self.notification = Some("No cleanup items selected".to_string());
            self.notification_time = Some(Instant::now());
            return;
        }
        let total_size: u64 = selection.iter().map(|c| c.size_bytes).sum();
        self.cleanup_pending = Some((selection, true));
        self.modal = Some(Modal::cleanup_confirm(
            self.cleanup_pending.as_ref().unwrap().0.len(),
            total_size,
            true,
        ));
    }

    fn cleanup_selection(&self) -> Vec<cleanup::rules::Candidate> {
        self.cleanup_candidates
            .iter()
            .filter(|c| self.cleanup_selected.contains(&c.path))
            .cloned()
            .collect()
    }

    pub fn start_delete(&mut self, path: &std::path::Path) -> Result<(), String> {
        let path_clone = path.to_path_buf();
        let (tx, rx) = mpsc::channel();
        let start_time = Instant::now();

        let handle =
            thread::spawn(
                move || match delete::delete_directory(&path_clone, Some(tx.clone())) {
                    Ok(result) => {
                        let duration_ms = start_time.elapsed().as_millis() as u64;
                        let success = result.errors.is_empty();

                        let log = logger::DeleteLog {
                            timestamp: Local::now().to_rfc3339(),
                            action: "delete".to_string(),
                            path: path_clone.display().to_string(),
                            size_bytes: result.total_bytes,
                            dry_run: false,
                            status: if success {
                                "success".to_string()
                            } else {
                                "error".to_string()
                            },
                            files_deleted: result.total_files,
                            duration_ms,
                            errors: if result.errors.is_empty() {
                                None
                            } else {
                                Some(result.errors.clone())
                            },
                        };

                        let _ = logger::write_log(&log);

                        if success {
                            let _ = tx.send(DeleteProgressUpdate::Complete {
                                total_bytes: result.total_bytes,
                                total_files: result.total_files,
                            });
                            Ok(())
                        } else {
                            let msg = result.errors.join("; ");
                            let _ = tx.send(DeleteProgressUpdate::Error(msg.clone()));
                            Err(msg)
                        }
                    }
                    Err(e) => {
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
                },
            );

        self.delete_thread = Some(handle);
        self.delete_rx = Some(rx);
        self.deleting_path = Some(path.to_path_buf());
        self.mode = AppMode::Deleting;
        self.delete_progress = Some(DeleteProgress {
            deleted_bytes: 0,
            total_bytes: 0,
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

                let total_size: u64 = files
                    .iter()
                    .filter_map(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum();

                let msg = format!(
                    "Dry-run: Would delete {} files ({:.1} MB)",
                    files.len(),
                    total_size as f64 / 1_048_576.0
                );

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
                        total_bytes as f64 / 1_048_576.0
                    );
                    self.notification = Some(msg);
                    self.notification_time = Some(Instant::now());
                    self.disk_space = platform::get_disk_space(&self.root_path);
                    // Remove deleted entry from tree (no rescan needed!)
                    if let Some(path) = self.deleting_path.take() {
                        self.remove_entry_from_tree(&path);
                    }
                }
                DeleteProgressUpdate::Error(e) => {
                    self.delete_progress = None;
                    self.delete_rx = None;
                    self.mode = AppMode::Browsing;
                    self.deleting_path = None; // Do not remove from tree on failure
                    let msg = format!("✗ Delete error: {}", e);
                    self.notification = Some(msg);
                    self.notification_time = Some(Instant::now());
                }
            }
        }
    }
}

fn find_node_mut<'a>(node: &'a mut FileNode, path: &Path) -> Option<&'a mut FileNode> {
    if node.path == path {
        return Some(node);
    }
    for child in &mut node.children {
        if path.starts_with(&child.path) {
            if let Some(found) = find_node_mut(child, path) {
                return Some(found);
            }
        }
    }
    None
}

/// Apply size delta to strict ancestors of `path` (not `path` itself).
fn apply_delta_to_ancestors(node: &mut FileNode, path: &Path, diff: i64) {
    if node.path == path {
        return;
    }
    node.size = (node.size as i64 + diff).max(0) as u64;
    for child in &mut node.children {
        if path.starts_with(&child.path) {
            if child.path == path {
                return;
            }
            apply_delta_to_ancestors(child, path, diff);
            return;
        }
    }
}

fn resort_along_path(node: &mut FileNode, path: &Path) {
    node.sort_children_by_size();
    if node.path == path {
        return;
    }
    for child in &mut node.children {
        if path.starts_with(&child.path) {
            resort_along_path(child, path);
            return;
        }
    }
}

#[derive(Debug, Clone)]
pub enum CleanupRow {
    Category {
        name: String,
    },
    Candidate {
        path: PathBuf,
        rule: String,
        pattern: String,
        size: u64,
    },
}

#[cfg(test)]
mod scan_merge_tests {
    use super::*;
    use std::sync::mpsc;
    use tempfile::tempdir;

    fn app_with_rx(rx: mpsc::Receiver<ScanProgress>) -> App {
        let mut app = App::create(PathBuf::from("/tmp"), false);
        app.scan_rx = Some(rx);
        app.is_scanning = true;
        app
    }

    #[test]
    fn listed_creates_tree_and_allows_enter() {
        let (tx, rx) = mpsc::sync_channel(16);
        let mut app = app_with_rx(rx);

        let root = PathBuf::from("/tmp/scan-root");
        let dir_a = root.join("a");
        let file_b = root.join("b.txt");

        tx.send(ScanProgress::Listed {
            path: root.clone(),
            children: vec![
                FileNode::new_dir("a".into(), dir_a.clone()),
                FileNode::new_file("b.txt".into(), file_b, 100),
            ],
        })
        .unwrap();

        app.update_scan_progress();

        assert!(app.tree.is_some());
        assert!(app.is_scanning);
        assert_eq!(app.entries_count(), 2);

        // Select directory "a" (largest-first: file 100 before empty dir 0, so index 1)
        let entries = app.get_display_entries();
        let a_idx = entries.iter().position(|e| e.name == "a").unwrap();
        app.selected_index = a_idx;
        app.enter_directory();
        assert_eq!(app.nav_stack.len(), 1);
        assert_eq!(app.get_current_path(), dir_a);
    }

    #[test]
    fn sized_updates_live_and_marks_complete() {
        let (tx, rx) = mpsc::sync_channel(16);
        let mut app = app_with_rx(rx);

        let root = PathBuf::from("/tmp/scan-root2");
        let dir_a = root.join("a");

        tx.send(ScanProgress::Listed {
            path: root.clone(),
            children: vec![FileNode::new_dir("a".into(), dir_a.clone())],
        })
        .unwrap();
        app.update_scan_progress();

        tx.send(ScanProgress::Sized {
            path: dir_a.clone(),
            size: 5000,
            complete: true,
        })
        .unwrap();
        app.update_scan_progress();

        let tree = app.tree.as_ref().unwrap();
        let child = tree.children.iter().find(|c| c.path == dir_a).unwrap();
        assert_eq!(child.size, 5000);
        assert!(child.complete);
        // Parent picks up delta
        assert_eq!(tree.size, 5000);

        tx.send(ScanProgress::Complete).unwrap();
        app.update_scan_progress();
        assert!(!app.is_scanning);
        assert!(app.scan_rx.is_none());
    }

    #[test]
    fn nav_selection_survives_resort_on_size_update() {
        let (tx, rx) = mpsc::sync_channel(16);
        let mut app = app_with_rx(rx);

        let root = PathBuf::from("/tmp/scan-root3");
        let small = root.join("small");
        let big = root.join("big");

        tx.send(ScanProgress::Listed {
            path: root.clone(),
            children: vec![
                FileNode::new_dir("small".into(), small.clone()),
                FileNode::new_dir("big".into(), big.clone()),
            ],
        })
        .unwrap();
        app.update_scan_progress();

        // Select "small"
        let idx = app
            .get_display_entries()
            .iter()
            .position(|e| e.name == "small")
            .unwrap();
        app.selected_index = idx;

        // big becomes larger → sort moves it first; selection should stay on small
        tx.send(ScanProgress::Sized {
            path: big,
            size: 10_000,
            complete: true,
        })
        .unwrap();
        app.update_scan_progress();

        let selected = &app.get_display_entries()[app.selected_index];
        assert_eq!(selected.name, "small");
        assert_eq!(selected.path, small);
    }

    #[test]
    fn incremental_scan_against_tempdir_merges_into_app() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("d")).unwrap();
        std::fs::write(root.join("d/f.txt"), "hello").unwrap();
        std::fs::write(root.join("x.txt"), "x").unwrap();

        let (tx, rx) = mpsc::sync_channel(128);
        let root_canon = root.canonicalize().unwrap();
        let handle = std::thread::spawn({
            let root_canon = root_canon.clone();
            move || {
                scan_tree_cancellable(&root_canon, Some(tx), None).unwrap();
            }
        });

        let mut app = App::create(root_canon.clone(), false);
        app.scan_rx = Some(rx);
        app.is_scanning = true;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.is_scanning && std::time::Instant::now() < deadline {
            app.update_scan_progress();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        handle.join().unwrap();
        app.update_scan_progress();

        assert!(!app.is_scanning);
        let tree = app.tree.as_ref().unwrap();
        assert!(tree.complete);
        assert_eq!(tree.path, root_canon);
        assert!(tree.children.len() >= 2);
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn setup_env(tmp: &tempfile::TempDir) -> mcdu_core::platform::PlatformPaths {
        let home = tmp.path().join("home");
        let cache = home.join(".cache");
        let config = home.join(".config");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&config).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        mcdu_core::platform::PlatformPaths::detect().unwrap()
    }

    fn write_simple_config(paths: &mcdu_core::platform::PlatformPaths) {
        let config_dir = paths.config_dir.join("mcdu");
        fs::create_dir_all(&config_dir).unwrap();
        let config = r#"
scan_paths = ["${CACHE_DIR}"]

[[rules]]
name = "all"
category = "test"
path = "${CACHE_DIR}"
pattern = "**/*"
enabled = true
risky = false
"#;
        fs::write(config_dir.join("cleanup.toml"), config).unwrap();
    }

    #[test]
    fn cleanup_scan_populates_candidates_and_saves_state() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        let paths = setup_env(&tmp);
        write_simple_config(&paths);

        let cache_file = paths.cache_dir.join("file.tmp");
        fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        let mut f = fs::File::create(&cache_file).unwrap();
        writeln!(f, "hello").unwrap();

        let mut app = App::new();
        app.start_cleanup_scan().unwrap();
        app.block_on_cleanup_scan();
        assert_eq!(app.cleanup_candidates.len(), 1);
        assert_eq!(app.cleanup_candidates[0].path, cache_file);

        let state_path = mcdu_core::config::default_config_paths(&paths).state_file;
        let state_contents = fs::read_to_string(state_path).unwrap();
        let state: mcdu_core::config::CleanupState = toml::from_str(&state_contents).unwrap();
        assert_eq!(state.selected.len(), 1);
    }

    #[test]
    fn cleanup_delete_removes_selected() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        let paths = setup_env(&tmp);
        write_simple_config(&paths);

        let cache_file = paths.cache_dir.join("file.tmp");
        fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        fs::write(&cache_file, "hello").unwrap();

        let mut app = App::new();
        app.start_cleanup_scan().unwrap();
        app.block_on_cleanup_scan();
        app.cleanup_selected.insert(cache_file.clone());

        app.start_cleanup_delete();
        app.handle_cleanup_modal_confirm(true);
        app.handle_cleanup_final_confirm(true);
        app.block_on_cleanup_delete();

        assert!(!cache_file.exists());
    }

    #[test]
    fn empty_cleanup_selection_does_not_delete_all() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempdir().unwrap();
        let paths = setup_env(&tmp);
        write_simple_config(&paths);

        let cache_file = paths.cache_dir.join("file.tmp");
        fs::create_dir_all(cache_file.parent().unwrap()).unwrap();
        fs::write(&cache_file, "hello").unwrap();

        let mut app = App::new();
        app.start_cleanup_scan().unwrap();
        app.block_on_cleanup_scan();
        assert!(!app.cleanup_candidates.is_empty());
        app.cleanup_selected.clear();

        app.start_cleanup_delete();
        assert!(app.modal.is_none());
        assert!(
            app.notification
                .as_deref()
                .unwrap_or("")
                .contains("No cleanup items selected")
        );
        assert!(cache_file.exists());
    }
}
