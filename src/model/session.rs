use std::path::PathBuf;

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum SessionStatus {
    Running,
    Paused,
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct CopiedFileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub delta: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SyncSession {
    pub job_id: Uuid,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub stats: SyncStats,
    pub errors: Vec<SyncError>,
    /// Mirror 模式下已删除的孤立文件路径
    pub deleted_paths: Vec<PathBuf>,
    /// 每个 worker 当前正在处理的文件
    pub active_workers: Vec<WorkerState>,
    /// 已复制的文件记录（用于日志）
    pub copied_log: Vec<CopiedFileEntry>,
    /// 孤立文件路径记录（Update 模式下目标端多余文件，用于日志）
    pub orphan_log: Vec<PathBuf>,
}

impl SyncSession {
    pub fn new(job_id: Uuid, concurrency: usize) -> Self {
        Self {
            job_id,
            status: SessionStatus::Running,
            started_at: Utc::now(),
            stats: SyncStats::default(),
            errors: Vec::new(),
            deleted_paths: Vec::new(),
            active_workers: vec![WorkerState::Idle; concurrency],
            copied_log: Vec::new(),
            orphan_log: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub total_files: u64,
    pub processed_files: u64,
    pub copied_files: u64,
    /// 使用差量传输的文件数
    pub delta_files: u64,
    pub skipped_files: u64,
    pub error_count: u64,
    pub total_bytes: u64,
    pub copied_bytes: u64,
    /// 差量传输节省的字节数
    pub saved_bytes: u64,
    /// Mirror 模式下删除的孤立文件数
    pub deleted_files: u64,
    /// 扫描到的孤立文件总数（目标端有、源端无）；Update 模式下不删除但仍统计
    pub orphan_files: u64,
    /// 当前传输速度（bytes/s），用于显示
    pub speed_bps: u64,
}

impl SyncStats {
    pub fn progress(&self) -> f32 {
        if self.total_files == 0 {
            return 0.0;
        }
        self.processed_files as f32 / self.total_files as f32
    }
}

#[derive(Debug, Clone)]
pub enum WorkerState {
    Idle,
    Copying { path: PathBuf, size: u64, done: u64 },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SyncError {
    pub timestamp: DateTime<Utc>,
    pub path: PathBuf,
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ErrorKind {
    AccessDenied,
    FileLocked,
    DiskFull,
    PathTooLong,
    IoError,
    Other,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::i18n::t;
        let s = match self {
            ErrorKind::AccessDenied => t("访问被拒绝", "Access denied"),
            ErrorKind::FileLocked => t("文件被锁定", "File locked"),
            ErrorKind::DiskFull => t("磁盘空间不足", "Disk full"),
            ErrorKind::PathTooLong => t("路径过长", "Path too long"),
            ErrorKind::IoError => t("IO 错误", "I/O error"),
            ErrorKind::Other => t("未知错误", "Unknown error"),
        };
        write!(f, "{}", s)
    }
}
