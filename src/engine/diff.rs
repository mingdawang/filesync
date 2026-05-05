use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::engine::scanner::ScanResult;

/// 单个文件的差异动作
#[derive(Debug, Clone, PartialEq)]
pub enum DiffAction {
    /// 目标端不存在，需要复制
    Create,
    /// 目标端存在但内容可能变化，需要覆盖
    Update,
    /// 内容相同，跳过
    Skip,
    /// 目标端多余文件（默认不删除，仅报告）
    Orphan,
}

#[derive(Debug, Clone)]
pub struct DiffEntry {
    /// 相对于源/目标根的路径（用于日志显示）
    pub relative_path: PathBuf,
    /// 源文件绝对路径
    pub source: PathBuf,
    /// 目标文件绝对路径
    pub destination: PathBuf,
    pub action: DiffAction,
    /// 源文件大小（字节）
    pub size: u64,
    /// 源文件最后修改时间
    pub modified: SystemTime,
}

/// 计算源和目标之间的差异列表
pub fn compute_diff(
    source_root: &Path,
    dest_root: &Path,
    source_scan: &ScanResult,
    dest_scan: &ScanResult,
) -> Vec<DiffEntry> {
    // 目标端索引：relative_path -> (size, modified)
    let dest_index: HashMap<&PathBuf, (u64, SystemTime)> = dest_scan
        .entries
        .iter()
        .map(|e| (&e.relative, (e.size, e.modified)))
        .collect();

    // 源端索引：用于检测孤立文件
    let source_index: std::collections::HashSet<&PathBuf> =
        source_scan.entries.iter().map(|e| &e.relative).collect();

    let mut result = Vec::new();

    // 源端 → Create / Update / Skip
    for src_file in &source_scan.entries {
        let dest_path = dest_root.join(&src_file.relative);

        let action = match dest_index.get(&src_file.relative) {
            None => DiffAction::Create,
            Some(&(dst_size, dst_modified)) => {
                if needs_update(src_file.size, src_file.modified, dst_size, dst_modified) {
                    DiffAction::Update
                } else {
                    DiffAction::Skip
                }
            }
        };

        result.push(DiffEntry {
            relative_path: src_file.relative.clone(),
            source: src_file.full_path.clone(),
            destination: dest_path,
            action,
            size: src_file.size,
            modified: src_file.modified,
        });
    }

    // 目标端 → Orphan（源端不存在的文件）
    for dst_file in &dest_scan.entries {
        if !source_index.contains(&dst_file.relative) {
            result.push(DiffEntry {
                relative_path: dst_file.relative.clone(),
                source: source_root.join(&dst_file.relative),
                destination: dst_file.full_path.clone(),
                action: DiffAction::Orphan,
                size: dst_file.size,
                modified: dst_file.modified,
            });
        }
    }

    result
}

/// 元数据比对：当大小不同或修改时间差超过容差时更新。
/// 在 metadata 模式下，源目录是权威来源，因此只要 mtime 明显不同，
/// 无论目标文件时间戳更早还是更晚，都需要重新同步。
fn needs_update(
    src_size: u64,
    src_modified: SystemTime,
    dst_size: u64,
    dst_modified: SystemTime,
) -> bool {
    if src_size != dst_size {
        return true;
    }

    const MTIME_TOLERANCE_SECS: u64 = 1;

    match src_modified.duration_since(dst_modified) {
        Ok(delta) => delta.as_secs() >= MTIME_TOLERANCE_SECS,
        Err(_) => dst_modified
            .duration_since(src_modified)
            .map(|delta| delta.as_secs() >= MTIME_TOLERANCE_SECS)
            .unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::needs_update;
    use std::time::{Duration, SystemTime};

    #[test]
    fn metadata_mode_updates_when_destination_is_newer_but_timestamp_differs() {
        let src = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let dst = SystemTime::UNIX_EPOCH + Duration::from_secs(20);

        assert!(needs_update(1024, src, 1024, dst));
    }

    #[test]
    fn metadata_mode_skips_when_size_matches_and_time_within_tolerance() {
        let src = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let dst = SystemTime::UNIX_EPOCH + Duration::from_millis(10_500);

        assert!(!needs_update(1024, src, 1024, dst));
    }
}
