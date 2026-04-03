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
    /// 传输速度更新（bytes/s）
    SpeedUpdate {
        bps: u64,
    },
    Paused,
    Resumed,
    /// 磁盘空间不足，需要用户介入
    DiskFull,
    /// 所有文件处理完毕
    Completed {
        stats: SyncStats,
        /// 新 USN 检查点（卷根路径 → (journal_id, next_usn)），仅无错误时有效
        usn_checkpoints: HashMap<String, (u64, i64)>,
    },
}
