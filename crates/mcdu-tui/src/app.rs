use crate::cleanup_ui::CleanupTab;
use crate::delete;
use crate::logger;
use crate::modal::Modal;
use crate::tree::{scan_tree_cancellable, FileNode, ScanProgress};
use chrono::Local;
use mcdu_core as cleanup;
use mcdu_core::platform::{self, DiskSpace};
use std::collections::HashSet;
use std::path::PathBuf;
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
    pub rescan_target: Option<(Vec<usize>, usize)>, // (nav_stack, child_index) for subtree rescan
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
            });
        }

        if let Some(node) = self.get_current_node() {
            for child in &node.children {
                entries.push(DisplayEntry {
                    name: child.name.clone(),
                    path: child.path.clone(),
                    size: child.size,
                    is_dir: child.is_dir,
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
        if self.tree.is_none() || self.is_scanning {
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

    /// Replace a subtree with a newly scanned one and update sizes
    fn replace_subtree(&mut self, nav_stack: Vec<usize>, child_idx: usize, new_tree: FileNode) {
        let Some(tree) = self.tree.as_mut() else {
            return;
        };

        // Capture paths along nav_stack for remapping after sort
        let mut path_stack: Vec<PathBuf> = Vec::new();
        {
            let mut node = &*tree;
            for &idx in &nav_stack {
                if idx >= node.children.len() {
                    return;
                }
                path_stack.push(node.children[idx].path.clone());
                node = &node.children[idx];
            }
            if child_idx >= node.children.len() {
                return;
            }
        }

        let replaced_path = {
            let mut node = &*tree;
            for &idx in &nav_stack {
                node = &node.children[idx];
            }
            node.children[child_idx].path.clone()
        };

        // Navigate to the parent node and replace
        let mut node = tree;
        for &idx in &nav_stack {
            node = &mut node.children[idx];
        }

        let old_size = node.children[child_idx].size;
        let new_size = new_tree.size;
        let size_diff = new_size as i64 - old_size as i64;
        let new_path = new_tree.path.clone();

        node.children[child_idx] = new_tree;
        node.children.sort_by(|a, b| b.size.cmp(&a.size));

        // Remap indices for the parent path after sort
        fn remap_path_stack(tree: &FileNode, paths: &[PathBuf]) -> Option<Vec<usize>> {
            let mut remapped = Vec::with_capacity(paths.len());
            let mut cursor = tree;
            for path in paths {
                let idx = cursor.children.iter().position(|c| &c.path == path)?;
                remapped.push(idx);
                cursor = &cursor.children[idx];
            }
            Some(remapped)
        }

        let tree_ref = self.tree.as_ref().unwrap();
        let remapped_parent = remap_path_stack(tree_ref, &path_stack).unwrap_or(nav_stack.clone());

        // Update sizes: root first, then each ancestor including the parent (like remove_entry_from_tree)
        if size_diff != 0 {
            let tree = self.tree.as_mut().unwrap();
            tree.size = (tree.size as i64 + size_diff).max(0) as u64;

            let mut parent = tree;
            for &idx in &remapped_parent {
                if idx >= parent.children.len() {
                    break;
                }
                parent = &mut parent.children[idx];
                parent.size = (parent.size as i64 + size_diff).max(0) as u64;
            }
        }

        // Remap live nav_stack paths that still exist
        let live_paths: Vec<PathBuf> = {
            let mut paths = Vec::new();
            let mut node = self.tree.as_ref().unwrap();
            for &idx in &self.nav_stack {
                if idx >= node.children.len() {
                    break;
                }
                paths.push(node.children[idx].path.clone());
                node = &node.children[idx];
            }
            paths
        };
        if let Some(remapped) = remap_path_stack(self.tree.as_ref().unwrap(), &live_paths) {
            self.nav_stack = remapped;
        }

        // Keep compiler quiet about unused (debugging hooks)
        let _ = (replaced_path, new_path);
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
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let handle = thread::spawn(move || {
            match scan_tree_cancellable(&path, Some(tx.clone()), Some(cancel_clone)) {
                Ok(tree) => {
                    let _ = tx.send(ScanProgress::Complete(tree));
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

        // Find the child index in the current node
        let Some(current) = self.get_current_node() else {
            return;
        };
        let Some(child_idx) = current.children.iter().position(|c| c.path == entry.path) else {
            return;
        };

        // Store where to put the result
        self.rescan_target = Some((self.nav_stack.clone(), child_idx));

        // Start scan of just this subtree
        if let Some(cancel) = self.scan_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let path = entry.path.clone();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let handle = thread::spawn(move || {
            match scan_tree_cancellable(&path, Some(tx.clone()), Some(cancel_clone)) {
                Ok(tree) => {
                    let _ = tx.send(ScanProgress::Complete(tree));
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

    pub fn update_scan_progress(&mut self) {
        if let Some(rx) = self.scan_rx.as_ref() {
            while let Ok(progress) = rx.try_recv() {
                match progress {
                    ScanProgress::Scanning {
                        files_scanned,
                        current_path,
                    } => {
                        self.scan_files_count = files_scanned;
                        self.scanning_path = Some(current_path);
                    }
                    ScanProgress::Complete(new_tree) => {
                        self.is_scanning = false;
                        self.scan_thread = None;
                        self.scan_rx = None;
                        self.scanning_path = None;

                        // Check if this is a subtree rescan
                        if let Some((nav_stack, child_idx)) = self.rescan_target.take() {
                            self.replace_subtree(nav_stack, child_idx, new_tree);
                            self.notification = Some("✓ Subtree rescanned".to_string());
                            self.notification_time = Some(Instant::now());
                        } else {
                            // Full tree scan
                            self.tree = Some(new_tree);
                            // Start splash fadeout if active
                            #[cfg(feature = "splash")]
                            if let Some(ref mut splash) = self.splash_state {
                                splash.start_fadeout();
                            }
                        }

                        self.disk_space = platform::get_disk_space(&self.root_path);
                        break;
                    }
                    ScanProgress::Error(e) => {
                        self.is_scanning = false;
                        self.scan_thread = None;
                        self.scan_rx = None;
                        self.notification = Some(format!("✗ Scan error: {}", e));
                        self.notification_time = Some(Instant::now());
                        break;
                    }
                }
            }
        }
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
