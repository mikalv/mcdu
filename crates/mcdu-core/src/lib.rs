//! mcdu-core - Core library for developer cleanup
//!
//! This crate provides:
//! - Rule-based cleanup scanning
//! - Project root detection
//! - Quarantine system with undo
//! - Parallel scanning support
//! - Platform-specific path resolution

pub mod config;
pub mod executor;
pub mod git;
pub mod parallel;
pub mod platform;
pub mod quarantine;
pub mod rules;
pub mod scanner;

// Re-exports for convenience
pub use config::{CleanupConfig, CleanupState};
pub use executor::{CleanupProgress, CleanupResult};
pub use parallel::{parallel_scan, ParallelScanConfig, ParallelScanner};
pub use platform::PlatformPaths;
pub use quarantine::{
    default_quarantine, Quarantine, QuarantineError, QuarantineManifest, QuarantineSettings,
};
pub use rules::{Candidate, MatchType, Rule};
pub use scanner::{group_by_category, scan, CategoryGroup, ScanProgress};
