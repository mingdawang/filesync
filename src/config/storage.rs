use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::model::config::AppConfig;

/// 返回配置文件路径：%APPDATA%\FileSync\config.json
pub fn config_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("FileSync").join("config.json")
}

/// 从磁盘加载配置，文件不存在时返回默认配置
pub fn load() -> Result<AppConfig> {
    let path = config_path();
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let config: AppConfig = serde_json::from_str(&raw)
        .with_context(|| "配置文件格式错误，请检查或删除后重试")?;
    Ok(migrate(config))
}

/// 原子写入配置到磁盘（先写临时文件再 rename）
pub fn save(config: &AppConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建配置目录: {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(&tmp, &content)
        .with_context(|| "写入临时配置文件失败")?;
    std::fs::rename(&tmp, &path)
        .with_context(|| "配置文件原子替换失败")?;
    Ok(())
}

/// 配置版本迁移（当前版本为 1，预留扩展）
fn migrate(mut config: AppConfig) -> AppConfig {
    if config.version < 1 {
        config.version = 1;
    }
    config
}
