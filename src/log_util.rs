use crate::output_manager::OutputManager;
use chrono::Utc;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
};

const LOG_FILENAME: &str = "learnchain-debug.log";

/// Append a timestamped line to the shared debug log. Errors are reported to stderr only.
pub fn log_debug(message: &str) {
    if let Err(err) = append_line(message) {
        eprintln!("[learnchain::log_util] failed to write debug log: {}", err);
    }
}

fn append_line(message: &str) -> io::Result<()> {
    let manager = OutputManager::new();
    let path = resolve_log_path(&manager)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "[{}] {}", Utc::now().to_rfc3339(), message)?;
    Ok(())
}

fn resolve_log_path(manager: &OutputManager) -> io::Result<PathBuf> {
    let mut dir = manager
        .output_directory()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    std::fs::create_dir_all(&dir)?;
    dir.push(LOG_FILENAME);
    Ok(dir)
}
