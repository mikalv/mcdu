use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum ModalType {
    ConfirmDelete {
        path: PathBuf,
        size: u64,
    },
    FinalConfirm {
        path: PathBuf,
        size: u64,
    },
    CleanupConfirm {
        items: usize,
        size: u64,
        dry_run: bool,
    },
    CleanupFinal {
        items: usize,
        size: u64,
        has_risky: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModalAction {
    Confirm,
    ConfirmWithGit,
    DryRun,
    Cancel,
}

pub struct Modal {
    pub modal_type: ModalType,
    pub selected_button: usize,
    pub buttons: Vec<(String, ModalAction)>,
}

impl Modal {
    pub fn confirm_delete(path: &Path, size: u64) -> Self {
        Modal {
            modal_type: ModalType::ConfirmDelete {
                path: path.to_path_buf(),
                size,
            },
            selected_button: 0,
            buttons: vec![
                ("Yes".to_string(), ModalAction::Confirm),
                ("No".to_string(), ModalAction::Cancel),
                ("Dry-run".to_string(), ModalAction::DryRun),
            ],
        }
    }

    pub fn final_confirm(path: &Path, size: u64) -> Self {
        Modal {
            modal_type: ModalType::FinalConfirm {
                path: path.to_path_buf(),
                size,
            },
            selected_button: 1, // Default to Cancel for safety
            buttons: vec![
                ("YES, DELETE".to_string(), ModalAction::Confirm),
                ("Cancel".to_string(), ModalAction::Cancel),
            ],
        }
    }

    pub fn cleanup_confirm(items: usize, size: u64, dry_run: bool) -> Self {
        Modal {
            modal_type: ModalType::CleanupConfirm {
                items,
                size,
                dry_run,
            },
            selected_button: 1,
            buttons: vec![
                ("Yes".to_string(), ModalAction::Confirm),
                ("No".to_string(), ModalAction::Cancel),
            ],
        }
    }

    pub fn cleanup_final(items: usize, size: u64, has_risky: bool) -> Self {
        Modal {
            modal_type: ModalType::CleanupFinal {
                items,
                size,
                has_risky,
            },
            selected_button: 2, // Default to Cancel for safety
            buttons: vec![
                ("YES, CLEANUP".to_string(), ModalAction::Confirm),
                ("YES + git gc".to_string(), ModalAction::ConfirmWithGit),
                ("Cancel".to_string(), ModalAction::Cancel),
            ],
        }
    }

    pub fn has_button(&self, label: &str) -> bool {
        self.buttons.iter().any(|(l, _)| l == label)
    }

    pub fn get_title(&self) -> String {
        match &self.modal_type {
            ModalType::ConfirmDelete { path, size } => {
                format!(
                    "Delete {} ({})? ",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    format_size(*size)
                )
            }
            ModalType::FinalConfirm { path, size } => {
                format!(
                    "FINAL CONFIRMATION - Delete {} ({})? ",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                    format_size(*size)
                )
            }
            ModalType::CleanupConfirm {
                items,
                size,
                dry_run,
            } => {
                format!(
                    "{} {} items ({})? ",
                    if *dry_run {
                        "Dry-run"
                    } else {
                        "Cleanup delete"
                    },
                    items,
                    format_size(*size)
                )
            }
            ModalType::CleanupFinal {
                items,
                size,
                has_risky: _,
            } => {
                format!(
                    "FINAL CONFIRMATION - Cleanup {} items ({})? ",
                    items,
                    format_size(*size)
                )
            }
        }
    }

    pub fn get_message(&self) -> String {
        match &self.modal_type {
            ModalType::ConfirmDelete { .. } => "This cannot be undone!".to_string(),
            ModalType::FinalConfirm { .. } => {
                "Really confirm? This is your last chance!".to_string()
            }
            ModalType::CleanupConfirm { dry_run, .. } => {
                if *dry_run {
                    "Show what would be cleaned without deleting.".to_string()
                } else {
                    "Proceed to final confirmation.".to_string()
                }
            }
            ModalType::CleanupFinal { has_risky, .. } => {
                if *has_risky {
                    "Selection includes RISKY items. git gc is opt-in (YES + git gc)."
                        .to_string()
                } else {
                    "Really delete selected cleanup items? git gc is opt-in (YES + git gc)."
                        .to_string()
                }
            }
        }
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.1} {}", size, UNITS[unit_idx])
}
