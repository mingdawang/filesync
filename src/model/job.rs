use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::config::CompareMethod;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SyncMode {
    #[default]
    Update,
    Mirror,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum DeleteMode {
    Direct,
    RecycleBin,
    #[default]
    FollowSystem,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum DeleteFallbackPolicy {
    #[default]
    Ask,
    Skip,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ReliabilityMode {
    Fast,
    #[default]
    Balanced,
    Safe,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum RunTrigger {
    #[default]
    Manual,
    Scheduled,
    Retry,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum RunResultStatus {
    #[default]
    Completed,
    Warning,
    Failed,
    Stopped,
    Missed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncJob {
    pub id: Uuid,
    pub name: String,
    pub sync_mode: SyncMode,
    pub concurrency: usize,
    #[serde(default)]
    pub compare_method: CompareMethod,
    #[serde(default)]
    pub delete_mode: DeleteMode,
    #[serde(default)]
    pub delete_fallback_policy: DeleteFallbackPolicy,
    #[serde(default)]
    pub reliability_mode: ReliabilityMode,
    pub folder_pairs: Vec<FolderPair>,
    pub exclusions: Vec<ExclusionRule>,
    pub engine_options: EngineOptions,
    #[serde(default)]
    pub schedule: SyncSchedule,
    #[serde(flatten, default, skip_serializing)]
    pub legacy_runtime: LegacyJobRuntime,
}

impl SyncJob {
    pub fn new(name: String, concurrency: usize) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            sync_mode: SyncMode::default(),
            concurrency: concurrency.max(1),
            compare_method: CompareMethod::default(),
            delete_mode: DeleteMode::default(),
            delete_fallback_policy: DeleteFallbackPolicy::default(),
            reliability_mode: ReliabilityMode::default(),
            folder_pairs: vec![FolderPair::new()],
            exclusions: vec![
                ExclusionRule::new("$RECYCLE.BIN/**".into()),
                ExclusionRule::new("System Volume Information/**".into()),
                ExclusionRule::new("Thumbs.db".into()),
                ExclusionRule::new("desktop.ini".into()),
                ExclusionRule::new("*.tmp".into()),
                ExclusionRule::new("~$*".into()),
            ],
            engine_options: EngineOptions::default(),
            schedule: SyncSchedule::default(),
            legacy_runtime: LegacyJobRuntime::default(),
        }
    }

    pub fn effective_copy_concurrency(&self) -> usize {
        match self.reliability_mode {
            ReliabilityMode::Fast => self.concurrency.max(1),
            ReliabilityMode::Balanced => self.concurrency.clamp(1, 4),
            ReliabilityMode::Safe => self.concurrency.clamp(1, 2),
            ReliabilityMode::Custom => self.concurrency.max(1),
        }
    }

    pub fn effective_hash_concurrency(&self) -> usize {
        match self.reliability_mode {
            ReliabilityMode::Fast => self.concurrency.clamp(1, 4),
            ReliabilityMode::Balanced => self.concurrency.clamp(1, 2),
            ReliabilityMode::Safe => 1,
            ReliabilityMode::Custom => self.concurrency.max(1),
        }
    }

    pub fn effective_delete_concurrency(&self) -> usize {
        match self.reliability_mode {
            ReliabilityMode::Fast => self.concurrency.max(1),
            ReliabilityMode::Balanced => self.concurrency.clamp(1, 2),
            ReliabilityMode::Safe => 1,
            ReliabilityMode::Custom => self.concurrency.max(1),
        }
    }

    pub fn effective_delta_concurrency(&self) -> usize {
        match self.reliability_mode {
            ReliabilityMode::Fast => self.concurrency.clamp(1, 2),
            ReliabilityMode::Balanced => 1,
            ReliabilityMode::Safe => 0,
            ReliabilityMode::Custom => self.concurrency.max(1),
        }
    }

    pub fn delta_threshold_bytes(&self) -> u64 {
        let configured = self.engine_options.delta_threshold_mb * 1024 * 1024;
        match self.reliability_mode {
            ReliabilityMode::Fast => configured,
            ReliabilityMode::Balanced => configured.max(16 * 1024 * 1024),
            ReliabilityMode::Safe => u64::MAX,
            ReliabilityMode::Custom => configured,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderPair {
    pub id: Uuid,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub enabled: bool,
}

impl FolderPair {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            source: PathBuf::new(),
            destination: PathBuf::new(),
            enabled: true,
        }
    }
}

impl Default for FolderPair {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExclusionRule {
    pub pattern: String,
    pub enabled: bool,
}

impl ExclusionRule {
    pub fn new(pattern: String) -> Self {
        Self {
            pattern,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOptions {
    pub unbuffered_threshold_mb: u64,
    pub delta_threshold_mb: u64,
    #[serde(default)]
    pub verify_after_copy: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            unbuffered_threshold_mb: 128,
            delta_threshold_mb: 4,
            verify_after_copy: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub copied: u64,
    pub skipped: u64,
    pub errors: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub elapsed_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunHistoryEntry {
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[serde(default)]
    pub trigger: RunTrigger,
    #[serde(default)]
    pub result: RunResultStatus,
    #[serde(default)]
    pub retry_attempt: u32,
    #[serde(default)]
    pub summary: Option<RunSummary>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSchedule {
    pub enabled: bool,
    pub interval_minutes: u32,
    #[serde(default = "default_retry_on_failure")]
    pub retry_on_failure: bool,
    #[serde(default = "default_max_retries")]
    pub max_retries: u8,
    #[serde(default = "default_retry_delay_minutes")]
    pub retry_delay_minutes: u32,
    #[serde(default = "default_pause_after_failures")]
    pub pause_after_failures: u8,
    #[serde(default)]
    pub risk_acknowledged: bool,
    #[serde(default = "default_delete_threshold")]
    pub delete_threshold: u64,
}

impl Default for SyncSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 60,
            retry_on_failure: default_retry_on_failure(),
            max_retries: default_max_retries(),
            retry_delay_minutes: default_retry_delay_minutes(),
            pause_after_failures: default_pause_after_failures(),
            risk_acknowledged: false,
            delete_threshold: default_delete_threshold(),
        }
    }
}

fn default_retry_on_failure() -> bool {
    true
}

fn default_max_retries() -> u8 {
    2
}

fn default_retry_delay_minutes() -> u32 {
    10
}

fn default_pause_after_failures() -> u8 {
    3
}

fn default_delete_threshold() -> u64 {
    1000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsnCheckpoint {
    pub journal_id: u64,
    pub next_usn: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LegacyJobRuntime {
    #[serde(default)]
    pub last_sync_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_run_summary: Option<RunSummary>,
    #[serde(default)]
    pub run_history: Vec<RunHistoryEntry>,
    #[serde(default)]
    pub consecutive_failures: u8,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub pause_reason: String,
}
