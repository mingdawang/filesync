use std::collections::HashMap;
use std::path::PathBuf;

use crate::model::session::SyncStats;

/// 同步引擎向 UI 线程发送的事件（通过 flume channel）
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SyncEvent {
    /// 扫描完成，开始执行
    Started {
        total_files: u64,
        total_bytes: u64,
    },
    /// 某个 worker 开始处理一个文件
    FileStarted {
        worker_id: usize,
        path: PathBuf,
        size: u64,
        /// true = 新建文件，false = 覆盖更新
        is_new: bool,
    },
    /// 文件复制字节进度
    FileProgress {
        worker_id: usize,
        bytes_done: u64,
    },
    /// 文件成功完成
    FileCompleted {
        worker_id: usize,
        path: PathBuf,
        size: u64,
        /// 是否使用了差量传输
        delta: bool,
        /// 差量传输节省的字节数（整体复制时为 0）
        saved_bytes: u64,
    },
    /// 文件跳过（无变化）
    FileSkipped {
        path: PathBuf,
    },
    /// 文件处理出错（不中断整体同步）
    FileError {
        path: PathBuf,
        message: String,
    },
    /// Mirror 模式下删除了目标端孤立文件
    FileDeleted {
        path: PathBuf,
    },
    /// Update 模式下检测到目标端孤立文件（不删除，仅记录）
    FileOrphan {
        path: PathBuf,
    },
    /// 传输速度更新（bytes/s）
    SpeedUpdate {
        bps: u64,
    },
    Paused,
    Resumed,
    /// 磁盘空间不足，需要用户介入
    DiskFull,
    /// 同步任务启动失败（例如 Tokio runtime 创建失败）
    StartFailed {
        message: String,
    },
    /// 所有文件处理完毕
    Completed {
        stats: SyncStats,
        /// 新 USN 检查点（卷根路径 → (journal_id, next_usn)）。
        /// 仅用于更新当前进程内的 `SyncJob::last_sync_checkpoints`，
        /// 不会持久化到 `config.json`。
        usn_checkpoints: HashMap<String, (u64, i64)>,
        /// true 表示用户主动停止，本次结果仅用于收尾和展示部分统计。
        was_stopped: bool,
    },
}
