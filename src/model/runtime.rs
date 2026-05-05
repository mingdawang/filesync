use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::job::{RunHistoryEntry, RunSummary, UsnCheckpoint};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JobStateRecord {
    pub job_id: Uuid,
    #[serde(default)]
    pub last_sync_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_run_summary: Option<RunSummary>,
    #[serde(default)]
    pub run_history: Vec<RunHistoryEntry>,
    #[serde(default)]
    pub schedule_runtime: ScheduleRuntimeState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScheduleRuntimeState {
    #[serde(default)]
    pub consecutive_failures: u8,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub pause_reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct JobTransientState {
    pub dirty: bool,
    pub last_sync_checkpoints: HashMap<String, UsnCheckpoint>,
}
