use std::path::PathBuf;
use std::time::SystemTime;

use crate::engine::diff::DiffAction;

/// 单条预览项：相对路径 + 操作类型 + 文件大小 + 修改时间
#[derive(Debug, Clone)]
pub struct PreviewEntry {
    pub relative_path: PathBuf,
    pub action: DiffAction,
    pub size: u64,
    pub modified: SystemTime,
}

/// 后台预览任务的状态
#[derive(Debug, Clone, Default)]
pub enum PreviewState {
    #[default]
    Idle,
    Loading,
    Ready(Vec<PreviewEntry>),
    Error(String),
}
