use crate::{config, output_manager::OutputManager};
use chrono::Utc;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

const LOG_FILENAME: &str = "learnchain-debug.log";
static RUNTIME_DEBUG_LOGGING: AtomicBool = AtomicBool::new(false);

pub fn enable_runtime_debug_logging() -> io::Result<PathBuf> {
    RUNTIME_DEBUG_LOGGING.store(true, Ordering::Relaxed);
    let manager = OutputManager::new();
    let path = resolve_log_path(&manager)?;
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)?;
    Ok(path)
}

/// Append a timestamped line to the shared debug log. Errors are reported to stderr only.
pub fn log_debug(message: &str) {
    if let Err(err) = append_line(message) {
        eprintln!("[learnchain::log_util] failed to write debug log: {}", err);
    }
}

fn append_line(message: &str) -> io::Result<()> {
    if !config::current().write_output_artifacts && !runtime_debug_logging_enabled() {
        return Ok(());
    }
    let manager = OutputManager::new();
    let path = resolve_log_path(&manager)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "[{}] {}", Utc::now().to_rfc3339(), message)?;
    Ok(())
}

fn resolve_log_path(manager: &OutputManager) -> io::Result<PathBuf> {
    let mut dir = manager.output_directory().map_err(io::Error::other)?;
    std::fs::create_dir_all(&dir)?;
    dir.push(LOG_FILENAME);
    Ok(dir)
}

fn runtime_debug_logging_enabled() -> bool {
    RUNTIME_DEBUG_LOGGING.load(Ordering::Relaxed)
}
