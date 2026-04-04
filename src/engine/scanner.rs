use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use globset::GlobSet;
use walkdir::WalkDir;

use crate::fs::long_path::maybe_extended;
use crate::log::LogLevel;
use crate::model::job::ExclusionRule;

pub struct ScanResult {
    pub entries: Vec<ScannedFile>,
}

impl ScanResult {
    pub fn empty() -> Self {
        Self { entries: Vec::new() }
    }
}

pub struct ScannedFile {
    /// 相对于扫描根目录的路径
    pub relative: PathBuf,
    /// 完整绝对路径
    pub full_path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// 根据排除规则构建 GlobSet
pub fn build_globset(rules: &[ExclusionRule]) -> GlobSet {
    let mut builder = globset::GlobSetBuilder::new();
    for rule in rules {
        if !rule.enabled {
            continue;
        }
        if let Ok(glob) = globset::GlobBuilder::new(&rule.pattern)
            .case_insensitive(true)
            .build()
        {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|e| {
        let msg = format!("exclusion GlobSet build failed (rules disabled): {}", e);
        crate::log::app_log(&msg, LogLevel::Error);
        GlobSet::empty()
    })
}

/// 扫描目录，返回所有文件（不含目录本身）
pub fn scan_directory(root: &Path, exclusions: &GlobSet) -> Result<ScanResult> {
    let mut entries = Vec::new();

    let scan_root = maybe_extended(root);
    for entry in WalkDir::new(&scan_root).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                crate::log::app_log(
                    &format!("scan: skipping entry (permission denied or I/O error): {}", e),
                    LogLevel::Error,
                );
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let full_path = entry.path().to_path_buf();
        let relative = match full_path.strip_prefix(&scan_root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };

        // 检查整个相对路径
        if exclusions.is_match(&relative) {
            continue;
        }
        // 检查路径中各个目录分量（如 .git 目录下的所有文件）
        if any_component_excluded(&relative, exclusions) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                crate::log::app_log(
                    &format!("scan: skipping file (metadata read failed): {} — {}", entry.path().display(), e),
                    LogLevel::Error,
                );
                continue;
            }
        };

        entries.push(ScannedFile {
            relative,
            full_path,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }

    Ok(ScanResult { entries })
}

/// 检查路径的任意分量是否被排除规则匹配
fn any_component_excluded(relative: &Path, exclusions: &GlobSet) -> bool {
    for component in relative.components() {
        let comp_path = Path::new(component.as_os_str());
        if exclusions.is_match(comp_path) {
            return true;
        }
    }
    false
}
