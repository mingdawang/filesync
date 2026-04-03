use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 同步模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SyncMode {
    /// 仅复制新增/变更文件，目标端多余文件保留（默认）
    #[default]
    Update,
    /// 复制新增/变更文件，并删除目标端孤立文件（使目标与源完全一致）
    Mirror,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncJob {
    pub id: Uuid,
    pub name: String,
    pub sync_mode: SyncMode,
    pub concurrency: usize,
    pub folder_pairs: Vec<FolderPair>,
    pub exclusions: Vec<ExclusionRule>,
    pub engine_options: EngineOptions,
    /// USN Journal 增量检查点，键为卷根路径（如 "C:\\"）。
    /// 仅保存在内存中，不持久化到磁盘，软件重启后自动清空，始终从全量扫描开始。
    #[serde(skip)]
    pub last_sync_checkpoints: HashMap<String, UsnCheckpoint>,
    pub last_sync_time: Option<DateTime<Utc>>,
    /// 上次运行统计摘要
    #[serde(default)]
    pub last_run_summary: Option<RunSummary>,
    /// 定时同步配置
    #[serde(default)]
    pub schedule: SyncSchedule,
}

impl SyncJob {
    pub fn new(name: String, concurrency: usize) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            sync_mode: SyncMode::default(),
            concurrency: concurrency.max(1),
            folder_pairs: vec![FolderPair::new()],
            exclusions: vec![
                ExclusionRule::new("Thumbs.db".into()),
                ExclusionRule::new("desktop.ini".into()),
                ExclusionRule::new("*.tmp".into()),
                ExclusionRule::new("~$*".into()),
            ],
            engine_options: EngineOptions::default(),
            last_sync_checkpoints: HashMap::new(),
            last_sync_time: None,
            last_run_summary: None,
            schedule: SyncSchedule::default(),
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
        Self { pattern, enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineOptions {
    /// 大于此值（MB）的文件使用无缓冲 IO
    pub unbuffered_threshold_mb: u64,
    /// 大于此值（MB）的文件使用差量传输，0 表示禁用
    pub delta_threshold_mb: u64,
    /// 复制完成后用 BLAKE3 校验目标文件与源文件一致性
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

/// 上次同步运行摘要（持久化到配置）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub copied: u64,
    pub skipped: u64,
    pub errors: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub elapsed_secs: u64,
}

/// 定时同步配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSchedule {
    /// 是否启用定时同步
    pub enabled: bool,
    /// 同步间隔（分钟），建议最小 5
    pub interval_minutes: u32,
}

impl Default for SyncSchedule {
    fn default() -> Self {
        Self { enabled: false, interval_minutes: 60 }
    }
}

/// USN Journal 检查点（用于增量哈希比对优化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsnCheckpoint {
    pub journal_id: u64,
    pub next_usn: i64,
}
