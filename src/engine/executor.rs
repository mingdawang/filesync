use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::bail;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use egui::Context;
use flume::Sender;
use tokio::sync::Semaphore;

use crate::engine::diff::DiffAction;
use crate::engine::events::{DeleteFallbackChoice, SyncEvent};
use crate::engine::scanner;
use crate::engine::{copier, diff, hash};
use crate::fs::volume::{detect_volume, VolumeCapabilities};
use crate::fs::usn_journal;
use crate::log::LogLevel;
use crate::model::config::CompareMethod;
use crate::model::job::{DeleteFallbackPolicy, DeleteMode, SyncJob, SyncMode};
use crate::model::session::{ErrorScope, SyncStats};

#[derive(Debug)]
enum DeleteOutcome {
    Deleted,
    Skipped,
}

struct PlannedDiff {
    diff: diff::DiffEntry,
    caps: Option<Arc<VolumeCapabilities>>,
}

/// 执行一次完整同步，通过 `tx` 向 UI 线程发送进度事件
pub async fn run_sync(
    job: SyncJob,
    tx: Sender<SyncEvent>,
    ctx: Context,
    stop: Arc<AtomicBool>,
) {
    let globset = scanner::build_globset(&job.exclusions);

    // ── Pre-scan: 采集 USN Journal 检查点与变化 FRN 集合 ─────────
    // new_checkpoints: 扫描前快照，作为本次同步后的新检查点
    let mut new_checkpoints: HashMap<String, (u64, i64)> = HashMap::new();
    // changed_frns: 自上次同步以来各卷变化的文件 FRN（空 = 无 USN 数据）
    let mut changed_frns: HashMap<String, HashSet<u64>> = HashMap::new();

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        for path in [&pair.source, &pair.destination] {
            if let Some(vol) = usn_journal::get_volume_key(path) {
                if new_checkpoints.contains_key(&vol) {
                    continue; // 该卷已处理
                }
                if let Some(info) = usn_journal::query_journal(&vol) {
                    // 若有上次完整同步的检查点且 journal 未重建，读取变化 FRN
                    if let Some(cp) = job.last_sync_checkpoints.get(&vol) {
                        if cp.journal_id == info.journal_id {
                            if cp.next_usn < info.next_usn {
                                // 有增量记录：读取变化 FRN。返回 None 表示读取失败
                                // （如 journal wraparound），此时不插入，回退到全量 hash 比对。
                                if let Some((frns, _)) = usn_journal::read_changed_frns(
                                    &vol,
                                    cp.next_usn,
                                    info.journal_id,
                                ) {
                                    changed_frns.insert(vol.clone(), frns);
                                }
                            } else {
                                // cp.next_usn == info.next_usn：自上次同步后该卷无任何变化，
                                // 插入空集合，usn_can_skip 会正确跳过所有 hash 比对。
                                changed_frns.insert(vol.clone(), std::collections::HashSet::new());
                            }
                        }
                    }
                    new_checkpoints.insert(vol, (info.journal_id, info.next_usn));
                }
            }
        }
    }

    // ── Step 1: 扫描所有文件夹对，计算差异列表 ──────────────────
    let mut all_diffs: Vec<PlannedDiff> = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut scan_error_count: u64 = 0;

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        if !pair.source.exists() {
            let _ = tx.send(SyncEvent::FileError {
                path: pair.source.clone(),
                message: "源目录不存在，已跳过".into(),
                scope: ErrorScope::Scan,
            });
            crate::log::app_log(
                &format!("sync skipped: source directory does not exist: {}", pair.source.display()),
                LogLevel::Error,
            );
            ctx.request_repaint();
            scan_error_count += 1;
            continue;
        }

        let src_scan = match scanner::scan_directory(&pair.source, &globset) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(SyncEvent::FileError {
                    path: pair.source.clone(),
                    message: format!("扫描源目录失败: {}", e),
                    scope: ErrorScope::Scan,
                });
                crate::log::app_log(
                    &format!("sync scan error: {} — {}", pair.source.display(), e),
                    LogLevel::Error,
                );
                ctx.request_repaint();
                scan_error_count += 1;
                continue;
            }
        };
        if report_scan_issues(&tx, &ctx, &src_scan.issues) {
            scan_error_count += src_scan.issues.len() as u64;
            continue;
        }

        if let Err(e) = sync_empty_directories(&pair.source, &pair.destination, &globset) {
            let _ = tx.send(SyncEvent::FileError {
                path: pair.destination.clone(),
                message: format!("创建目标目录失败: {}", e),
                scope: ErrorScope::Scan,
            });
            crate::log::app_log(
                &format!(
                    "sync directory creation error: {} -> {} — {}",
                    pair.source.display(),
                    pair.destination.display(),
                    e
                ),
                LogLevel::Error,
            );
            ctx.request_repaint();
            scan_error_count += 1;
            continue;
        }

        let pair_caps = Arc::new(detect_destination_volume(&pair.destination));

        let dst_scan = if pair.destination.exists() {
            match scanner::scan_directory(&pair.destination, &globset) {
                Ok(scan) => scan,
                Err(e) => {
                    let _ = tx.send(SyncEvent::FileError {
                        path: pair.destination.clone(),
                        message: format!("扫描目标目录失败: {}", e),
                        scope: ErrorScope::Scan,
                    });
                    crate::log::app_log(
                        &format!("sync destination scan error: {} — {}", pair.destination.display(), e),
                        LogLevel::Error,
                    );
                    ctx.request_repaint();
                    scan_error_count += 1;
                    continue;
                }
            }
        } else {
            scanner::ScanResult::empty()
        };
        if report_scan_issues(&tx, &ctx, &dst_scan.issues) {
            scan_error_count += dst_scan.issues.len() as u64;
            continue;
        }

        let diffs = diff::compute_diff(&pair.source, &pair.destination, &src_scan, &dst_scan);

        for d in diffs {
            if d.action == DiffAction::Create || d.action == DiffAction::Update {
                total_bytes += d.size;
            }
            all_diffs.push(PlannedDiff {
                diff: d,
                caps: Some(pair_caps.clone()),
            });
        }
    }

    // ── Step 1b: Hash 精确比对（减少不必要的复制）────────────────
    if job.compare_method == CompareMethod::Hash {
        // Phase 1: USN-optimized skips — no I/O, apply immediately
        for planned in all_diffs.iter_mut() {
            if planned.diff.action == DiffAction::Update
                && usn_can_skip(&planned.diff.source, &planned.diff.destination, &changed_frns)
            {
                planned.diff.action = DiffAction::Skip;
                total_bytes = total_bytes.saturating_sub(planned.diff.size);
            }
        }

        // Phase 2: Hash comparisons run in parallel via JoinSet
        let mut hash_tasks: tokio::task::JoinSet<(usize, bool)> = tokio::task::JoinSet::new();
        let hash_parallelism = job.concurrency.max(1);
        let hash_sem = Arc::new(Semaphore::new(hash_parallelism));
        for (i, planned) in all_diffs.iter().enumerate() {
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
                (i, same)
            });
        }
        while let Some(result) = hash_tasks.join_next().await {
            if let Ok((i, true)) = result {
                let planned = &mut all_diffs[i];
                if planned.diff.action == DiffAction::Update {
                    planned.diff.action = DiffAction::Skip;
                    total_bytes = total_bytes.saturating_sub(planned.diff.size);
                }
            }
        }
    }

    let total_files = all_diffs
        .iter()
        .filter(|d| d.diff.action != DiffAction::Orphan)
        .count() as u64;

    let _ = tx.send(SyncEvent::Started { total_files, total_bytes });
    ctx.request_repaint();

    // 从 engine_options 读取阈值配置
    let delta_threshold = job.engine_options.delta_threshold_mb * 1024 * 1024;
    let unbuffered_threshold = job.engine_options.unbuffered_threshold_mb * 1024 * 1024;
    let verify_after_copy = job.engine_options.verify_after_copy;

    // ── Step 2: 速度计 ─────────────────────────────────────────────
    let bytes_transferred = Arc::new(AtomicU64::new(0));
    {
        let bytes_ref = bytes_transferred.clone();
        let tx_speed = tx.clone();
        let ctx_speed = ctx.clone();
        let stop_speed = stop.clone();

        tokio::spawn(async move {
            let mut prev: u64 = 0;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if stop_speed.load(Ordering::Relaxed) {
                    break;
                }
                let current = bytes_ref.load(Ordering::Relaxed);
                let bps = current.saturating_sub(prev);
                prev = current;
                let _ = tx_speed.send(SyncEvent::SpeedUpdate { bps });
                ctx_speed.request_repaint();
            }
        });
    }

    // ── Step 3: 并发执行复制 ──────────────────────────────────────
    let sem = Arc::new(Semaphore::new(job.concurrency.max(1)));

    let copied = Arc::new(AtomicU64::new(0));
    let copy_errors = Arc::new(AtomicU64::new(0));
    let delete_errors = Arc::new(AtomicU64::new(0));
    let skipped = Arc::new(AtomicU64::new(0));
    let saved_bytes = Arc::new(AtomicU64::new(0));
    let delta_count = Arc::new(AtomicU64::new(0));

    // 收集孤立文件路径（Mirror 模式需要）
    let mut orphan_paths: Vec<PathBuf> = Vec::new();

    let mut task_index: usize = 0;
    let mut handles = Vec::new();

    for planned in all_diffs {
        let d = planned.diff;
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match d.action {
            DiffAction::Orphan => {
                orphan_paths.push(d.destination.clone());
                if job.sync_mode != SyncMode::Mirror {
                    let _ = tx.send(SyncEvent::FileOrphan { path: d.destination.clone() });
                }
                continue;
            }

            DiffAction::Skip => {
                skipped.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(SyncEvent::FileSkipped { path: d.relative_path });
                continue;
            }

            DiffAction::Create | DiffAction::Update => {
                let worker_id = task_index % job.concurrency.max(1);
                task_index += 1;

                // acquire_owned() only fails if the semaphore is closed;
                // we never close it, so this should never happen in practice.
                let permit = match sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        crate::log::app_log("semaphore closed unexpectedly, aborting copy loop", LogLevel::Error);
                        break;
                    }
                };
                let tx2 = tx.clone();
                let ctx2 = ctx.clone();
                let stop2 = stop.clone();
                let copied2 = copied.clone();
                let errors2 = copy_errors.clone();
                let saved2 = saved_bytes.clone();
                let delta2 = delta_count.clone();
                let caps2 = planned.caps.clone();
                let bytes2 = bytes_transferred.clone();
                let size = d.size;
                let use_delta = delta_threshold > 0 && size >= delta_threshold;

                let handle = tokio::spawn(async move {
                    let _permit = permit;

                    let _ = tx2.send(SyncEvent::FileStarted {
                        worker_id,
                        path: d.source.clone(),
                        size,
                        is_new: d.action == DiffAction::Create,
                    });
                    ctx2.request_repaint();

                    let src = d.source.clone();
                    let dst = d.destination.clone();
                    let tx3 = tx2.clone();
                    let stop3 = stop2.clone();

                    let result = tokio::task::spawn_blocking(move || {
                        if use_delta && dst.exists() {
                            // 尝试 delta，失败回退到普通复制
                            match crate::engine::delta::delta_sync(
                                &src, &dst, worker_id, &tx3, &stop3,
                            ) {
                                Ok((_, sv)) => {
                                    saved2.fetch_add(sv, Ordering::Relaxed);
                                    delta2.fetch_add(1, Ordering::Relaxed);
                                    Ok((true, sv))
                                }
                                Err(_) if stop3.load(Ordering::Relaxed) => {
                                    bail!("已停止")
                                }
                                Err(_) => {
                                    copier::copy_file_with_caps(
                                        &src, &dst, worker_id, size, &tx3, &stop3,
                                        caps2.as_deref(), verify_after_copy,
                                        unbuffered_threshold,
                                    )
                                    .map(|_| (false, 0u64))
                                }
                            }
                        } else {
                            copier::copy_file_with_caps(
                                &src, &dst, worker_id, size, &tx3, &stop3,
                                caps2.as_deref(), verify_after_copy,
                                unbuffered_threshold,
                            )
                            .map(|_| (false, 0u64))
                        }
                    })
                    .await;

                    match result {
                        Ok(Ok((delta_used, sv))) => {
                            copied2.fetch_add(1, Ordering::Relaxed);
                            bytes2.fetch_add(size, Ordering::Relaxed);
                            let _ = tx2.send(SyncEvent::FileCompleted {
                                worker_id,
                                path: d.relative_path,
                                size,
                                delta: delta_used,
                                saved_bytes: sv,
                            });
                        }
                        Ok(Err(_e)) if stop2.load(Ordering::Relaxed) => {}
                        Ok(Err(e)) => {
                            errors2.fetch_add(1, Ordering::Relaxed);
                            let _ = tx2.send(SyncEvent::FileError {
                                path: d.source.clone(),
                                message: e.to_string(),
                                scope: ErrorScope::Copy,
                            });
                            let _ = tx2.send(SyncEvent::WorkerFinished { worker_id });
                            crate::log::app_log(
                                &format!("sync copy error: {} — {}", d.source.display(), e),
                                LogLevel::Error,
                            );
                        }
                        Err(_e) if stop2.load(Ordering::Relaxed) => {}
                        Err(e) => {
                            errors2.fetch_add(1, Ordering::Relaxed);
                            let _ = tx2.send(SyncEvent::FileError {
                                path: d.source.clone(),
                                message: format!("task panic: {}", e),
                                scope: ErrorScope::Copy,
                            });
                            let _ = tx2.send(SyncEvent::WorkerFinished { worker_id });
                            crate::log::app_log(
                                &format!("sync task panic: {} — {}", d.source.display(), e),
                                LogLevel::Error,
                            );
                        }
                    }

                    ctx2.request_repaint();
                });

                handles.push(handle);
            }
        }
    }

    for h in handles {
        let _ = h.await;
    }

    // ── Step 4: Mirror 模式——删除孤立文件和目录 ──────────────────
    let deleted = Arc::new(AtomicU64::new(0));
    let mut orphan_dir_count: u64 = 0;

    if job.sync_mode == SyncMode::Mirror && !stop.load(Ordering::Relaxed) {
        // 收集需要尝试清理的父目录（用 HashSet 去重）
        let mut parent_dirs: HashSet<PathBuf> = HashSet::new();

        let mut delete_targets: Vec<(PathBuf, bool)> = Vec::new();

        for path in &orphan_paths {
            if let Some(parent) = path.parent() {
                parent_dirs.insert(parent.to_path_buf());
            }
            delete_targets.push((path.clone(), false));
        }

        if !stop.load(Ordering::Relaxed) {
            for pair in &job.folder_pairs {
                if !pair.enabled {
                    continue;
                }
                let dirs = collect_orphan_dirs(&pair.source, &pair.destination);
                orphan_dir_count += dirs.len() as u64;
                for dir in dirs {
                    delete_targets.push((dir, true));
                }
            }
        }

        let delete_sem = Arc::new(Semaphore::new(job.concurrency.max(1)));
        let mut delete_handles = Vec::new();

        for (delete_index, (path, is_dir)) in delete_targets.into_iter().enumerate() {
            if stop.load(Ordering::Relaxed) {
                break;
            }

            let permit = match delete_sem.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    crate::log::app_log("delete semaphore closed unexpectedly", LogLevel::Error);
                    break;
                }
            };

            let worker_id = delete_index % job.concurrency.max(1);
            let tx_delete = tx.clone();
            let ctx_delete = ctx.clone();
            let stop_delete = stop.clone();
            let stop_for_delete = stop_delete.clone();
            let deleted_count = deleted.clone();
            let error_count = delete_errors.clone();
            let delete_mode = job.delete_mode.clone();
            let delete_fallback_policy = job.delete_fallback_policy.clone();
            let allow_delete_prompt = !job.schedule.enabled;
            let tx_prompt = tx.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit;

                let _ = tx_delete.send(SyncEvent::DeleteStarted {
                    worker_id,
                    path: path.clone(),
                    is_dir,
                });
                ctx_delete.request_repaint();

                let delete_path = path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    delete_with_mode(
                        &delete_path,
                        &delete_mode,
                        &delete_fallback_policy,
                        allow_delete_prompt,
                        is_dir,
                        &tx_prompt,
                        &stop_for_delete,
                    )
                })
                .await;

                match result {
                    Ok(Ok(DeleteOutcome::Deleted)) => {
                        deleted_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_delete.send(SyncEvent::FileDeleted { worker_id, path });
                    }
                    Ok(Ok(DeleteOutcome::Skipped)) => {
                        let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                    }
                    Ok(Err(_e)) if stop_delete.load(Ordering::Relaxed) => {
                        let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                    }
                    Ok(Err(e)) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_delete.send(SyncEvent::FileError {
                            path: path.clone(),
                            message: if is_dir {
                                format!("删除孤立目录失败: {}", e)
                            } else {
                                format!("删除孤立文件失败: {}", e)
                            },
                            scope: ErrorScope::Delete,
                        });
                        let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                    }
                    Err(e) if stop_delete.load(Ordering::Relaxed) => {
                        let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                        crate::log::app_log(
                            &format!("delete task cancelled during stop: {}", e),
                            LogLevel::Info,
                        );
                    }
                    Err(e) => {
                        error_count.fetch_add(1, Ordering::Relaxed);
                        let _ = tx_delete.send(SyncEvent::FileError {
                            path: path.clone(),
                            message: format!("delete task panic: {}", e),
                            scope: ErrorScope::Delete,
                        });
                        let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                        crate::log::app_log(
                            &format!("delete task panic: {} — {}", path.display(), e),
                            LogLevel::Error,
                        );
                    }
                }

                ctx_delete.request_repaint();
            });

            delete_handles.push(handle);
        }

        for handle in delete_handles {
            let _ = handle.await;
        }

        let mut dirs_sorted: Vec<PathBuf> = parent_dirs.into_iter().collect();
        dirs_sorted.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
        for dir in dirs_sorted {
            let _ = std::fs::remove_dir(&dir);
        }
    }

    // ── Step 4b: Update 模式——统计孤立目录（不删除，仅上报）────────
    if job.sync_mode != SyncMode::Mirror && !stop.load(Ordering::Relaxed) {
        for pair in &job.folder_pairs {
            if !pair.enabled { continue; }
            for dir in collect_orphan_dirs(&pair.source, &pair.destination) {
                orphan_dir_count += 1;
                let _ = tx.send(SyncEvent::FileOrphan { path: dir });
            }
        }
    }

    // ── Step 5: 发送完成事件 ──────────────────────────────────────
    let final_copied = copied.load(Ordering::Relaxed);
    let final_copy_errors = copy_errors.load(Ordering::Relaxed);
    let final_delete_errors = delete_errors.load(Ordering::Relaxed);
    let final_errors = scan_error_count + final_copy_errors + final_delete_errors;
    let final_skipped = skipped.load(Ordering::Relaxed);
    let final_deleted = deleted.load(Ordering::Relaxed);
    let was_stopped = stop.load(Ordering::Relaxed);

    let stats = SyncStats {
        total_files,
        total_bytes,
        copied_files: final_copied,
        copied_bytes: bytes_transferred.load(Ordering::Relaxed),
        skipped_files: final_skipped,
        error_count: final_errors,
        scan_error_count,
        copy_error_count: final_copy_errors,
        delete_error_count: final_delete_errors,
        processed_files: final_copied + final_skipped + final_copy_errors,
        delta_files: delta_count.load(Ordering::Relaxed),
        saved_bytes: saved_bytes.load(Ordering::Relaxed),
        deleted_files: final_deleted,
        orphan_files: orphan_paths.len() as u64 + orphan_dir_count,
        speed_bps: 0,
    };

    // 中止时不保存检查点：被中止意味着部分文件未扫描，检查点位置之后的变化可能漏记。
    // 有错误但未中止时仍保存检查点：复制错误不影响 USN 有效性，下次同步仍可增量跳过未变化文件。
    let usn_checkpoints = if !was_stopped {
        new_checkpoints
    } else {
        HashMap::new()
    };

    let _ = tx.send(SyncEvent::Completed {
        stats,
        usn_checkpoints,
        was_stopped,
    });
    ctx.request_repaint();
}

// ─────────────────────────────────────────────────────────────────
// 孤立目录辅助函数
// ─────────────────────────────────────────────────────────────────

/// 收集目标目录中源端不存在的孤立子目录，由深到浅排序（先删子目录，再删父目录）。
pub(crate) fn collect_orphan_dirs(src: &Path, dst: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for entry in walkdir::WalkDir::new(dst).follow_links(false).min_depth(1) {
        let entry = match entry { Ok(e) => e, Err(_) => continue };
        if !entry.file_type().is_dir() { continue; }
        let dir_path = entry.path().to_path_buf();
        let relative = match dir_path.strip_prefix(dst) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if !src.join(relative).exists() {
            dirs.push(dir_path);
        }
    }
    dirs.sort_by(|a, b| b.components().count().cmp(&a.components().count()));
    dirs
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

fn report_scan_issues(tx: &Sender<SyncEvent>, ctx: &Context, issues: &[scanner::ScanIssue]) -> bool {
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
    ctx.request_repaint();
    true
}

fn detect_destination_volume(path: &Path) -> VolumeCapabilities {
    let vol_path = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|pp| pp.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf())
    };
    detect_volume(&vol_path)
}

// ─────────────────────────────────────────────────────────────────
// USN Journal 辅助函数
// ─────────────────────────────────────────────────────────────────

/// 从路径快速推断卷根（如 "C:\\"），不调用系统 API。
///
/// 仅适用于标准 `X:\...` 驱动器路径。`usn_can_skip` 使用此函数而非
/// `usn_journal::get_volume_key`（Win32 API），是因为它运行在并发任务中，
/// 频繁的系统调用开销不值得；而 `changed_frns` 的键由 pre-scan 阶段的
/// `get_volume_key` 生成，两者对标准路径返回相同的 "X:\\" 格式，行为一致。
fn vol_root_simple(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        Some(format!("{}\\", &s[..2]))
    } else {
        None
    }
}

/// 判断源文件和目标文件是否均未变化，可安全跳过哈希比对并转为 Skip。
///
/// 条件：
/// 1. `changed_frns` 非空（有上次同步检查点，即有 USN 增量数据）
/// 2. 两端文件的 FRN 均可获取（NTFS/ReFS 支持）
/// 3. 两端 FRN 均不在变化集中（自上次同步后均未修改）
fn usn_can_skip(
    src: &Path,
    dst: &Path,
    changed_frns: &HashMap<String, HashSet<u64>>,
) -> bool {
    if changed_frns.is_empty() {
        return false;
    }

    let src_vol = match vol_root_simple(src) { Some(v) => v, None => return false };
    let dst_vol = match vol_root_simple(dst) { Some(v) => v, None => return false };

    let src_set = match changed_frns.get(&src_vol) { Some(s) => s, None => return false };
    let dst_set = match changed_frns.get(&dst_vol) { Some(s) => s, None => return false };

    // FRN 查询仅对 Update 文件（数量远少于全量），开销可接受
    let src_frn = match usn_journal::get_file_index(src) { Some(f) => f, None => return false };
    let dst_frn = match usn_journal::get_file_index(dst) { Some(f) => f, None => return false };

    !src_set.contains(&src_frn) && !dst_set.contains(&dst_frn)
}

fn delete_with_mode(
    path: &std::path::Path,
    mode: &DeleteMode,
    fallback_policy: &DeleteFallbackPolicy,
    allow_prompt: bool,
    is_dir: bool,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<DeleteOutcome, String> {
    match mode {
        DeleteMode::Direct => delete_direct(path).map(|_| DeleteOutcome::Deleted),
        DeleteMode::RecycleBin => trash::delete(path)
            .map(|_| DeleteOutcome::Deleted)
            .map_err(|e| e.to_string()),
        DeleteMode::FollowSystem => match trash::delete(path) {
            Ok(()) => Ok(DeleteOutcome::Deleted),
            Err(e) => request_delete_confirmation(
                path,
                fallback_policy,
                allow_prompt,
                is_dir,
                e.to_string(),
                tx,
                stop,
            ),
        },
    }
}

fn request_delete_confirmation(
    path: &Path,
    fallback_policy: &DeleteFallbackPolicy,
    allow_prompt: bool,
    is_dir: bool,
    reason: String,
    tx: &Sender<SyncEvent>,
    stop: &Arc<AtomicBool>,
) -> Result<DeleteOutcome, String> {
    if stop.load(Ordering::Relaxed) {
        return Err("已停止".into());
    }

    match fallback_policy {
        DeleteFallbackPolicy::Skip => return Ok(DeleteOutcome::Skipped),
        DeleteFallbackPolicy::Fail => {
            return Err(format!("回收站删除失败: {}", reason));
        }
        DeleteFallbackPolicy::Ask => {}
    }

    if !allow_prompt {
        return Err(format!(
            "回收站删除失败且当前为无人值守运行，无法等待确认: {}",
            reason
        ));
    }

    let (response_tx, response_rx) = std::sync::mpsc::channel();
    let item_label = if is_dir { "directory" } else { "file" };
    tx.send(SyncEvent::DeleteFallbackRequired {
        path: path.to_path_buf(),
        is_dir,
        message: format!("Failed to move {} to Recycle Bin: {}", item_label, reason),
        response: response_tx,
    })
    .map_err(|e| e.to_string())?;

    match response_rx.recv() {
        Ok(DeleteFallbackChoice::DirectDelete) => {
            delete_direct(path).map(|_| DeleteOutcome::Deleted)
        }
        Ok(DeleteFallbackChoice::Skip) => Ok(DeleteOutcome::Skipped),
        Ok(DeleteFallbackChoice::StopSync) => Err("已停止".into()),
        Err(_) => Err("delete confirmation channel closed".into()),
    }
}

fn delete_direct(path: &std::path::Path) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{request_delete_confirmation, sync_empty_directories, DeleteOutcome};
    use crate::engine::events::{DeleteFallbackChoice, SyncEvent};
    use crate::model::job::DeleteFallbackPolicy;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn follow_system_delete_requires_confirmation_before_direct_delete() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("orphan.txt");
        std::fs::write(&path, b"data").unwrap();

        let (tx, rx) = flume::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_path = path.clone();

        let handle = std::thread::spawn(move || {
            request_delete_confirmation(
                &worker_path,
                &DeleteFallbackPolicy::Ask,
                true,
                false,
                "Recycle Bin unavailable".into(),
                &tx,
                &worker_stop,
            )
        });

        let event = rx.recv().unwrap();
        let SyncEvent::DeleteFallbackRequired { response, .. } = event else {
            panic!("expected DeleteFallbackRequired");
        };
        response.send(DeleteFallbackChoice::Skip).unwrap();

        let result = handle.join().unwrap().unwrap();
        assert!(matches!(result, DeleteOutcome::Skipped));
        assert!(path.exists());
    }

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
    fn delete_fallback_policy_skip_does_not_block_or_delete() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("orphan.txt");
        std::fs::write(&path, b"data").unwrap();
        let (tx, _rx) = flume::unbounded();
        let stop = Arc::new(AtomicBool::new(false));

        let result = request_delete_confirmation(
            &path,
            &DeleteFallbackPolicy::Skip,
            false,
            false,
            "Recycle Bin unavailable".into(),
            &tx,
            &stop,
        )
        .unwrap();

        assert!(matches!(result, DeleteOutcome::Skipped));
        assert!(path.exists());
    }

    #[test]
    fn unattended_ask_policy_fails_instead_of_blocking() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("orphan.txt");
        std::fs::write(&path, b"data").unwrap();
        let (tx, _rx) = flume::unbounded();
        let stop = Arc::new(AtomicBool::new(false));

        let err = request_delete_confirmation(
            &path,
            &DeleteFallbackPolicy::Ask,
            false,
            false,
            "Recycle Bin unavailable".into(),
            &tx,
            &stop,
        )
        .unwrap_err();

        assert!(err.contains("无人值守"));
    }
}
