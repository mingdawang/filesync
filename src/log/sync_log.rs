use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::i18n::is_zh;
use crate::model::session::{CopiedFileEntry, SyncError, SyncStats};

pub struct SyncLogData<'a> {
    pub job_name: &'a str,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub stats: &'a SyncStats,
    pub copied_log: &'a [CopiedFileEntry],
    pub deleted_log: &'a [PathBuf],
    pub orphan_log: &'a [PathBuf],
    pub errors: &'a [SyncError],
}

/// Write sync log to `%LOCALAPPDATA%\FileSync\logs\{job}_{datetime}.log`, returns log file path.
pub fn write_sync_log(data: &SyncLogData) -> std::io::Result<PathBuf> {
    let log_dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        crate::log::app_log(&format!("failed to create sync log directory: {}", e), crate::log::LogLevel::Error);
    }

    let safe_name = sanitize_filename(data.job_name);
    let timestamp = data.started_at.format("%Y-%m-%d_%H-%M-%S");
    let filename = format!("{}_{}.log", safe_name, timestamp);
    let path = log_dir.join(&filename);

    match std::fs::File::create(&path) {
        Ok(mut f) => {
            if let Err(e) = write_content(&mut f, data) {
                crate::log::app_log(&format!("failed to write sync log: {}", e), crate::log::LogLevel::Error);
            }
            Ok(path)
        }
        Err(e) => {
            crate::log::app_log(&format!("failed to create sync log file {}: {}", path.display(), e), crate::log::LogLevel::Error);
            Err(e)
        }
    }
}

pub fn log_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            crate::log::app_log(
                "LOCALAPPDATA env var not set, using current directory for sync logs",
                crate::log::LogLevel::Error,
            );
            PathBuf::from(".")
        });
    base.join("FileSync").join("logs")
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn write_content(f: &mut std::fs::File, data: &SyncLogData) -> std::io::Result<()> {
    let zh = is_zh();
    let elapsed = (data.finished_at - data.started_at).num_seconds().max(0);

    writeln!(f, "========================================")?;
    if zh {
        writeln!(f, "FileSync 同步日志")?;
    } else {
        writeln!(f, "FileSync Sync Log")?;
    }
    writeln!(f, "========================================")?;

    if zh {
        writeln!(f, "任务名称: {}", data.job_name)?;
        writeln!(f, "开始时间: {}", data.started_at.format("%Y-%m-%d %H:%M:%S UTC"))?;
        writeln!(f, "结束时间: {}", data.finished_at.format("%Y-%m-%d %H:%M:%S UTC"))?;
        writeln!(f, "耗时:     {} 秒", elapsed)?;
    } else {
        writeln!(f, "Job:      {}", data.job_name)?;
        writeln!(f, "Started:  {}", data.started_at.format("%Y-%m-%d %H:%M:%S UTC"))?;
        writeln!(f, "Finished: {}", data.finished_at.format("%Y-%m-%d %H:%M:%S UTC"))?;
        writeln!(f, "Elapsed:  {}s", elapsed)?;
    }
    writeln!(f)?;

    if zh {
        writeln!(f, "[摘要]")?;
        writeln!(f, "  复制文件: {}", data.stats.copied_files)?;
        writeln!(f, "  跳过文件: {}", data.stats.skipped_files)?;
        writeln!(f, "  删除项目: {}", data.stats.deleted_files)?;
        if data.stats.orphan_files > 0 {
            writeln!(f, "  孤立项目: {}", data.stats.orphan_files)?;
        }
        writeln!(f, "  错误数量: {}", data.stats.error_count)?;
        writeln!(f, "  传输字节: {}", format_bytes(data.stats.copied_bytes))?;
        if data.stats.delta_files > 0 {
            writeln!(
                f,
                "  差量传输: {} 个文件，节省 {}",
                data.stats.delta_files,
                format_bytes(data.stats.saved_bytes)
            )?;
        }
    } else {
        writeln!(f, "[Summary]")?;
        writeln!(f, "  Copied:   {}", data.stats.copied_files)?;
        writeln!(f, "  Skipped:  {}", data.stats.skipped_files)?;
        writeln!(f, "  Deleted:  {}", data.stats.deleted_files)?;
        if data.stats.orphan_files > 0 {
            writeln!(f, "  Orphans:  {}", data.stats.orphan_files)?;
        }
        writeln!(f, "  Errors:   {}", data.stats.error_count)?;
        writeln!(f, "  Bytes:    {}", format_bytes(data.stats.copied_bytes))?;
        if data.stats.delta_files > 0 {
            writeln!(
                f,
                "  Delta:    {} file(s), saved {}",
                data.stats.delta_files,
                format_bytes(data.stats.saved_bytes)
            )?;
        }
    }
    writeln!(f)?;

    if zh {
        writeln!(f, "[操作文件列表]")?;
    } else {
        writeln!(f, "[File Operations]")?;
    }

    for entry in data.copied_log {
        if entry.delta {
            writeln!(
                f,
                "  [{}] {} ({}) [Delta]",
                if zh { "复制" } else { "Copy" },
                entry.path.display(),
                format_bytes(entry.size)
            )?;
        } else {
            writeln!(
                f,
                "  [{}] {} ({})",
                if zh { "复制" } else { "Copy" },
                entry.path.display(),
                format_bytes(entry.size)
            )?;
        }
    }

    for path in data.deleted_log {
        writeln!(
            f,
            "  [{}] {}",
            if zh { "删除" } else { "Delete" },
            path.display()
        )?;
    }

    for path in data.orphan_log {
        writeln!(
            f,
            "  [{}] {}",
            if zh { "孤立" } else { "Orphan" },
            path.display()
        )?;
    }

    for err in data.errors {
        writeln!(
            f,
            "  [{}] {} - {}: {}",
            if zh { "错误" } else { "Error" },
            err.path.display(),
            err.kind,
            err.message
        )?;
    }

    Ok(())
}
