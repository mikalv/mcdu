use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteLog {
    pub timestamp: String,
    pub action: String,
    pub path: String,
    pub size_bytes: u64,
    pub dry_run: bool,
    pub status: String,
    pub files_deleted: u64,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<String>>,
}

pub fn write_log(log: &DeleteLog) -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = get_log_dir()?;
    fs::create_dir_all(&log_dir)?;

    let filename = format!("delete-{}.log", Local::now().format("%Y-%m-%d"));
    let log_path = log_dir.join(filename);

    // Append to log file (one JSON per line)
    let json_line = serde_json::to_string(log)? + "\n";
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    file.write_all(json_line.as_bytes())?;

    Ok(())
}

pub fn get_log_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    Ok(PathBuf::from(home).join(".mcdu").join("logs"))
}
