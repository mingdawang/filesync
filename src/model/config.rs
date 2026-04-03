use serde::{Deserialize, Serialize};

use super::job::SyncJob;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub settings: AppSettings,
    pub jobs: Vec<SyncJob>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            settings: AppSettings::default(),
            jobs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub default_concurrency: usize,
    pub compare_method: CompareMethod,
    pub theme: Theme,
    /// 点击窗口关闭按钮时的行为（仅在托盘可用时有意义）
    #[serde(default)]
    pub close_action: CloseAction,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_concurrency: 4,
            compare_method: CompareMethod::Metadata,
            theme: Theme::System,
            close_action: CloseAction::Ask,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CompareMethod {
    Metadata,
    Hash,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// 点击窗口关闭按钮（X）时的行为。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CloseAction {
    /// 每次询问用户
    #[default]
    Ask,
    /// 始终最小化到托盘
    MinimizeToTray,
    /// 始终退出程序
    Quit,
}
