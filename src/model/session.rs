use std::path::PathBuf;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::model::job::RunTrigger;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorScope {
    Scan,
    Copy,
    Delete,
}

impl std::fmt::Display for ErrorScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            ErrorScope::Scan => "scan",
            ErrorScope::Copy => "copy",
            ErrorScope::Delete => "delete",
        };
        write!(f, "{}", label)
    }
}

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
    pub job_name: String,
    pub trigger: RunTrigger,
    pub retry_attempt: u32,
    pub status: SessionStatus,
    pub started_at: DateTime<Utc>,
    pub started_at_instant: std::time::Instant,
    pub stats: SyncStats,
    pub errors: Vec<SyncError>,
    pub deleted_paths: Vec<PathBuf>,
    pub active_workers: Vec<WorkerState>,
    pub copied_log: Vec<CopiedFileEntry>,
    pub orphan_log: Vec<PathBuf>,
    pub last_speed_sample_at: std::time::Instant,
    pub last_speed_sample_bytes: u64,
}

impl SyncSession {
    pub fn new(
        job_id: Uuid,
        job_name: String,
        concurrency: usize,
        trigger: RunTrigger,
        retry_attempt: u32,
    ) -> Self {
        Self {
            job_id,
            job_name,
            trigger,
            retry_attempt,
            status: SessionStatus::Running,
            started_at: Utc::now(),
            started_at_instant: std::time::Instant::now(),
            stats: SyncStats::default(),
            errors: Vec::new(),
            deleted_paths: Vec::new(),
            active_workers: vec![WorkerState::Idle; concurrency],
            copied_log: Vec::new(),
            orphan_log: Vec::new(),
            last_speed_sample_at: std::time::Instant::now(),
            last_speed_sample_bytes: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub total_files: u64,
    pub processed_files: u64,
    pub copied_files: u64,
    pub delta_files: u64,
    pub skipped_files: u64,
    pub error_count: u64,
    pub scan_error_count: u64,
    pub copy_error_count: u64,
    pub delete_error_count: u64,
    pub total_bytes: u64,
    pub copied_bytes: u64,
    pub saved_bytes: u64,
    pub deleted_files: u64,
    pub orphan_files: u64,
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
    Copying {
        path: PathBuf,
        size: u64,
        done: u64,
        is_new: bool,
    },
    Deleting {
        path: PathBuf,
        is_dir: bool,
    },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SyncError {
    pub timestamp: DateTime<Utc>,
    pub path: PathBuf,
    pub scope: ErrorScope,
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
