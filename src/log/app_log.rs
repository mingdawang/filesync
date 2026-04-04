use std::io::Write;
use std::path::PathBuf;

use chrono::Utc;

/// Log severity levels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Error,
}

/// Log file path: `%LOCALAPPDATA%\FileSync\app_2026-04-04.log`
fn log_file_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(".")
    });
    let date = Utc::now().format("%Y-%m-%d");
    base.join("FileSync").join(format!("app_{}.log", date))
}

/// Write an application log line (append mode, with timestamp).
///
/// - **dev build** (`debug_assertions`): logs both `Info` and `Error`.
/// - **release build**: logs only `Error`.
///
/// All messages should be in English.
pub fn app_log(msg: &str, level: LogLevel) {
    if level == LogLevel::Info && !cfg!(debug_assertions) {
        return;
    }

    let tag = match level {
        LogLevel::Info => "INFO ",
        LogLevel::Error => "ERROR",
    };

    let path = log_file_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            if cfg!(debug_assertions) {
                eprintln!("[{}] failed to create log dir: {}", tag, e);
            }
            return;
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let ts = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f UTC");
        let _ = writeln!(f, "[{}] [{}] {}", ts, tag, msg);
    }
}
