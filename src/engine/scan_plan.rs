use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::Context;
use flume::Sender;
use tokio::sync::Semaphore;

use crate::engine::diff::DiffAction;
use crate::engine::events::SyncEvent;
use crate::engine::{diff, hash, messages, scanner};
use crate::fs::usn_journal;
use crate::fs::volume::{detect_volume, VolumeCapabilities};
use crate::log::LogLevel;
use crate::model::config::CompareMethod;
use crate::model::job::{SyncJob, UsnCheckpoint};
use crate::model::session::ErrorScope;

#[derive(Clone)]
pub(crate) struct PlannedDiff {
    pub(crate) diff: diff::DiffEntry,
    pub(crate) caps: Option<Arc<VolumeCapabilities>>,
}

pub(crate) struct SyncPlan {
    pub(crate) diffs: Vec<PlannedDiff>,
    pub(crate) total_bytes: u64,
    pub(crate) scan_error_count: u64,
    pub(crate) new_checkpoints: HashMap<String, (u64, i64)>,
}

pub(crate) async fn build_sync_plan(
    job: &SyncJob,
    checkpoints: &HashMap<String, UsnCheckpoint>,
    tx: &Sender<SyncEvent>,
    ctx: &Context,
) -> SyncPlan {
    let globset = scanner::build_globset(&job.exclusions);
    let (new_checkpoints, changed_frns) = collect_usn_state(job, checkpoints);

    let mut diffs: Vec<PlannedDiff> = Vec::new();
    let mut total_bytes = 0;
    let mut scan_error_count = 0;

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        if !pair.source.exists() {
            let _ = tx.send(SyncEvent::FileError {
                path: pair.source.clone(),
                message: messages::source_directory_skipped().into(),
                scope: ErrorScope::Scan,
            });
            crate::log::app_log(
                &format!(
                    "sync skipped: source directory does not exist: {}",
                    pair.source.display()
                ),
                LogLevel::Error,
            );
            scan_error_count += 1;
            continue;
        }

        let src_scan = match scanner::scan_directory(&pair.source, &globset) {
            Ok(scan) => scan,
            Err(err) => {
                let _ = tx.send(SyncEvent::FileError {
                    path: pair.source.clone(),
                    message: messages::scan_source_failed(&err.to_string()),
                    scope: ErrorScope::Scan,
                });
                crate::log::app_log(
                    &format!(
                        "sync scan error: {} - {}",
                        pair.source.display(),
                        err
                    ),
                    LogLevel::Error,
                );
                scan_error_count += 1;
                continue;
            }
        };
        if report_scan_issues(tx, ctx, &src_scan.issues) {
            scan_error_count += src_scan.issues.len() as u64;
            continue;
        }

        if let Err(err) = sync_empty_directories(&pair.source, &pair.destination, &globset) {
            let _ = tx.send(SyncEvent::FileError {
                path: pair.destination.clone(),
                message: messages::create_destination_failed(&err.to_string()),
                scope: ErrorScope::Scan,
            });
            crate::log::app_log(
                &format!(
                    "sync directory creation error: {} -> {} - {}",
                    pair.source.display(),
                    pair.destination.display(),
                    err
                ),
                LogLevel::Error,
            );
            scan_error_count += 1;
            continue;
        }

        let pair_caps = Arc::new(detect_destination_volume(&pair.destination));
        let dst_scan = if pair.destination.exists() {
            match scanner::scan_directory(&pair.destination, &globset) {
                Ok(scan) => scan,
                Err(err) => {
                    let _ = tx.send(SyncEvent::FileError {
                        path: pair.destination.clone(),
                        message: messages::scan_destination_failed(&err.to_string()),
                        scope: ErrorScope::Scan,
                    });
                    crate::log::app_log(
                        &format!(
                            "sync destination scan error: {} - {}",
                            pair.destination.display(),
                            err
                        ),
                        LogLevel::Error,
                    );
                    scan_error_count += 1;
                    continue;
                }
            }
        } else {
            scanner::ScanResult::empty()
        };
        if report_scan_issues(tx, ctx, &dst_scan.issues) {
            scan_error_count += dst_scan.issues.len() as u64;
            continue;
        }

        for diff in diff::compute_diff(&pair.source, &pair.destination, &src_scan, &dst_scan) {
            if matches!(diff.action, DiffAction::Create | DiffAction::Update) {
                total_bytes += diff.size;
            }
            diffs.push(PlannedDiff {
                diff,
                caps: Some(pair_caps.clone()),
            });
        }
    }

    if job.compare_method == CompareMethod::Hash {
        apply_hash_compare(job, &mut diffs, &mut total_bytes, &changed_frns).await;
    }

    SyncPlan {
        diffs,
        total_bytes,
        scan_error_count,
        new_checkpoints,
    }
}

pub(crate) fn count_non_orphan_files(diffs: &[PlannedDiff]) -> u64 {
    diffs.iter()
        .filter(|entry| entry.diff.action != DiffAction::Orphan)
        .count() as u64
}

pub(crate) fn collect_orphan_dirs(src: &Path, dst: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for entry in walkdir::WalkDir::new(dst).follow_links(false).min_depth(1) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_dir() {
            continue;
        }
        let dir_path = entry.path().to_path_buf();
        let relative = match dir_path.strip_prefix(dst) {
            Ok(relative) => relative,
            Err(_) => continue,
        };
        if !src.join(relative).exists() {
            dirs.push(dir_path);
        }
    }
    dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
    dirs
}

fn collect_usn_state(
    job: &SyncJob,
    checkpoints: &HashMap<String, UsnCheckpoint>,
) -> (HashMap<String, (u64, i64)>, HashMap<String, HashSet<u64>>) {
    let mut new_checkpoints = HashMap::new();
    let mut changed_frns = HashMap::new();

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        for path in [&pair.source, &pair.destination] {
            if let Some(vol) = usn_journal::get_volume_key(path) {
                if new_checkpoints.contains_key(&vol) {
                    continue;
                }
                if let Some(info) = usn_journal::query_journal(&vol) {
                    if let Some(cp) = checkpoints.get(&vol) {
                        if cp.journal_id == info.journal_id {
                            if cp.next_usn < info.next_usn {
                                if let Some((frns, _)) =
                                    usn_journal::read_changed_frns(&vol, cp.next_usn, info.journal_id)
                                {
                                    changed_frns.insert(vol.clone(), frns);
                                }
                            } else {
                                changed_frns.insert(vol.clone(), HashSet::new());
                            }
                        }
                    }
                    new_checkpoints.insert(vol, (info.journal_id, info.next_usn));
                }
            }
        }
    }

    (new_checkpoints, changed_frns)
}

async fn apply_hash_compare(
    job: &SyncJob,
    diffs: &mut [PlannedDiff],
    total_bytes: &mut u64,
    changed_frns: &HashMap<String, HashSet<u64>>,
) {
    for planned in diffs.iter_mut() {
        if planned.diff.action == DiffAction::Update
            && usn_can_skip(&planned.diff.source, &planned.diff.destination, changed_frns)
        {
            planned.diff.action = DiffAction::Skip;
            *total_bytes = total_bytes.saturating_sub(planned.diff.size);
        }
    }

    let mut hash_tasks: tokio::task::JoinSet<(usize, bool)> = tokio::task::JoinSet::new();
    let hash_sem = Arc::new(Semaphore::new(job.concurrency.max(1)));
    for (idx, planned) in diffs.iter().enumerate() {
        if planned.diff.action != DiffAction::Update {
            continue;
        }
        let src = planned.diff.source.clone();
        let dst = planned.diff.destination.clone();
        let permit = match hash_sem.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };
        hash_tasks.spawn(async move {
            let _permit = permit;
            let same = tokio::task::spawn_blocking(move || {
                matches!(
                    (hash::hash_file(&src), hash::hash_file(&dst)),
                    (Some(sh), Some(dh)) if sh == dh
                )
            })
            .await
            .unwrap_or(false);
            (idx, same)
        });
    }

    while let Some(result) = hash_tasks.join_next().await {
        if let Ok((idx, true)) = result {
            let planned = &mut diffs[idx];
            if planned.diff.action == DiffAction::Update {
                planned.diff.action = DiffAction::Skip;
                *total_bytes = total_bytes.saturating_sub(planned.diff.size);
            }
        }
    }
}

fn sync_empty_directories(
    src: &Path,
    dst: &Path,
    exclusions: &globset::GlobSet,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;

    for entry in walkdir::WalkDir::new(src).follow_links(false).min_depth(1) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_dir() {
            continue;
        }

        let relative = match entry.path().strip_prefix(src) {
            Ok(relative) => relative,
            Err(_) => continue,
        };
        if is_excluded(relative, exclusions) {
            continue;
        }

        std::fs::create_dir_all(dst.join(relative))?;
    }

    Ok(())
}

fn is_excluded(relative: &Path, exclusions: &globset::GlobSet) -> bool {
    if exclusions.is_match(relative) {
        return true;
    }
    relative
        .components()
        .any(|component| exclusions.is_match(Path::new(component.as_os_str())))
}

fn report_scan_issues(tx: &Sender<SyncEvent>, _ctx: &Context, issues: &[scanner::ScanIssue]) -> bool {
    if issues.is_empty() {
        return false;
    }
    for issue in issues {
        let _ = tx.send(SyncEvent::FileError {
            path: issue.path.clone(),
            message: issue.message.clone(),
            scope: ErrorScope::Scan,
        });
    }
    true
}

fn detect_destination_volume(path: &Path) -> VolumeCapabilities {
    let vol_path = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf())
    };
    detect_volume(&vol_path)
}

fn usn_can_skip(
    src: &Path,
    dst: &Path,
    changed_frns: &HashMap<String, HashSet<u64>>,
) -> bool {
    if changed_frns.is_empty() {
        return false;
    }

    let src_vol = match vol_root_simple(src) {
        Some(vol) => vol,
        None => return false,
    };
    let dst_vol = match vol_root_simple(dst) {
        Some(vol) => vol,
        None => return false,
    };

    let src_set = match changed_frns.get(&src_vol) {
        Some(set) => set,
        None => return false,
    };
    let dst_set = match changed_frns.get(&dst_vol) {
        Some(set) => set,
        None => return false,
    };

    let src_frn = match usn_journal::get_file_index(src) {
        Some(frn) => frn,
        None => return false,
    };
    let dst_frn = match usn_journal::get_file_index(dst) {
        Some(frn) => frn,
        None => return false,
    };

    !src_set.contains(&src_frn) && !dst_set.contains(&dst_frn)
}

fn vol_root_simple(path: &Path) -> Option<String> {
    let path_str = path.to_str()?;
    let bytes = path_str.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        Some(format!("{}\\", &path_str[..2]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{collect_orphan_dirs, sync_empty_directories};

    #[test]
    fn sync_empty_directories_creates_nested_empty_directories() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(src.path().join("a/b/c")).unwrap();
        let exclusions = crate::engine::scanner::build_globset(&[]);

        sync_empty_directories(src.path(), dst.path(), &exclusions).unwrap();

        assert!(dst.path().join("a/b/c").is_dir());
    }

    #[test]
    fn collect_orphan_dirs_returns_deepest_first() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        std::fs::create_dir_all(src.path().join("keep")).unwrap();
        std::fs::create_dir_all(dst.path().join("keep")).unwrap();
        std::fs::create_dir_all(dst.path().join("orphan/a/b")).unwrap();

        let dirs = collect_orphan_dirs(src.path(), dst.path());

        assert_eq!(dirs.len(), 3);
        assert!(dirs[0].ends_with("orphan\\a\\b") || dirs[0].ends_with("orphan/a/b"));
    }
}
