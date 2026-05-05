use std::path::PathBuf;

use std::io::Write as _;

use anyhow::{Context, Result};

use crate::log::LogLevel;
use crate::model::config::AppConfig;
use crate::model::runtime::{JobStateRecord, ScheduleRuntimeState};

/// Returns config file path: `%LOCALAPPDATA%\FileSync\config.json`
pub fn config_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| {
        crate::log::app_log("LOCALAPPDATA env var not set, using current directory for config", LogLevel::Error);
        PathBuf::from(".")
    });
    base.join("FileSync").join("config.json")
}

/// Load config from disk, returns default if file not found
pub fn load() -> Result<AppConfig> {
    let path = config_path();
    crate::log::app_log(&format!("loading config from: {}", path.display()), LogLevel::Info);
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::log::app_log("config file not found, using defaults", LogLevel::Info);
            return Ok(AppConfig::default());
        }
        Err(e) => {
            return Err(e)
                .with_context(|| format!("failed to read config file: {}", path.display()))
        }
    };
    let config: AppConfig = serde_json::from_str(&raw)
        .with_context(|| "config file format error, please check or delete and retry")?;
    crate::log::app_log(&format!("config loaded, {} job(s)", config.jobs.len()), LogLevel::Info);
    Ok(migrate(config))
}

/// Atomic write config to disk (write temp file then rename)
pub fn save(config: &AppConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory: {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(config)?;
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| "failed to create temp config file")?;
        file.write_all(content.as_bytes())
            .with_context(|| "failed to write temp config file")?;
        // fsync before rename: prevents partial-write surviving a power loss
        file.sync_all()
            .with_context(|| "failed to fsync temp config file")?;
    }
    crate::fs::replace::replace_file(&tmp, &path)
        .with_context(|| "atomic config file rename failed")?;
    crate::log::app_log(&format!("config saved to {}", path.display()), LogLevel::Info);
    Ok(())
}

/// Config version migration (current version 1, reserved for future)
fn migrate(mut config: AppConfig) -> AppConfig {
    if config.version < 1 {
        config.version = 1;
    }

    let mut states = std::mem::take(&mut config.job_states);
    for job in &config.jobs {
        if states.iter().any(|state| state.job_id == job.id) {
            continue;
        }
        states.push(JobStateRecord {
            job_id: job.id,
            last_sync_time: job.legacy_runtime.last_sync_time,
            last_run_summary: job.legacy_runtime.last_run_summary.clone(),
            run_history: job.legacy_runtime.run_history.clone(),
            schedule_runtime: ScheduleRuntimeState {
                consecutive_failures: job.legacy_runtime.consecutive_failures,
                paused: job.legacy_runtime.paused,
                pause_reason: job.legacy_runtime.pause_reason.clone(),
            },
        });
    }
    states.retain(|state| config.jobs.iter().any(|job| job.id == state.job_id));
    config.job_states = states;
    config
}
