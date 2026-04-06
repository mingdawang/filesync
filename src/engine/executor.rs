use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::bail;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use egui::Context;
use flume::Sender;
use tokio::sync::Semaphore;

use crate::engine::diff::DiffAction;
use crate::engine::events::SyncEvent;
use crate::engine::scanner;
use crate::engine::{copier, diff, hash};
use crate::fs::volume::{detect_volume, VolumeCapabilities};
use crate::fs::usn_journal;
use crate::log::LogLevel;
use crate::model::config::CompareMethod;
use crate::model::job::{SyncJob, SyncMode};
use crate::model::session::SyncStats;

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
                        if cp.journal_id == info.journal_id && cp.next_usn < info.next_usn {
                            let (frns, _) = usn_journal::read_changed_frns(
                                &vol,
                                cp.next_usn,
                                info.journal_id,
                            );
                            changed_frns.insert(vol.clone(), frns);
                        }
                    }
                    new_checkpoints.insert(vol, (info.journal_id, info.next_usn));
                }
            }
        }
    }

    // ── Step 1: 扫描所有文件夹对，计算差异列表 ──────────────────
    let mut all_diffs = Vec::new();
    let mut total_bytes: u64 = 0;

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        if !pair.source.exists() {
            let _ = tx.send(SyncEvent::FileError {
                path: pair.source.clone(),
                message: "源目录不存在，已跳过".into(),
            });
            crate::log::app_log(
                &format!("sync skipped: source directory does not exist: {}", pair.source.display()),
                LogLevel::Error,
            );
            ctx.request_repaint();
            continue;
        }

        let src_scan = match scanner::scan_directory(&pair.source, &globset) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(SyncEvent::FileError {
                    path: pair.source.clone(),
                    message: format!("扫描源目录失败: {}", e),
                });
                crate::log::app_log(
                    &format!("sync scan error: {} — {}", pair.source.display(), e),
                    LogLevel::Error,
                );
                ctx.request_repaint();
                continue;
            }
        };

        let dst_scan = if pair.destination.exists() {
            scanner::scan_directory(&pair.destination, &globset)
                .unwrap_or_else(|_| scanner::ScanResult::empty())
        } else {
            scanner::ScanResult::empty()
        };

        let diffs = diff::compute_diff(&pair.source, &pair.destination, &src_scan, &dst_scan);

        for d in &diffs {
            if d.action == DiffAction::Create || d.action == DiffAction::Update {
                total_bytes += d.size;
            }
        }

        all_diffs.extend(diffs);
    }

    // ── Step 1b: Hash 精确比对（减少不必要的复制）────────────────
    if job.compare_method == CompareMethod::Hash {
        // Phase 1: USN-optimized skips — no I/O, apply immediately
        for d in all_diffs.iter_mut() {
            if d.action == DiffAction::Update
                && usn_can_skip(&d.source, &d.destination, &changed_frns)
            {
                d.action = DiffAction::Skip;
                total_bytes = total_bytes.saturating_sub(d.size);
            }
        }

        // Phase 2: Hash comparisons run in parallel via JoinSet
        let mut hash_tasks: tokio::task::JoinSet<(usize, bool)> = tokio::task::JoinSet::new();
        for (i, d) in all_diffs.iter().enumerate() {
            if d.action != DiffAction::Update {
                continue;
            }
            let src = d.source.clone();
            let dst = d.destination.clone();
            hash_tasks.spawn_blocking(move || {
                let same = matches!(
                    (hash::hash_file(&src), hash::hash_file(&dst)),
                    (Some(sh), Some(dh)) if sh == dh
                );
                (i, same)
            });
        }
        while let Some(result) = hash_tasks.join_next().await {
            if let Ok((i, true)) = result {
                let d = &mut all_diffs[i];
                if d.action == DiffAction::Update {
                    d.action = DiffAction::Skip;
                    total_bytes = total_bytes.saturating_sub(d.size);
                }
            }
        }
    }

    let total_files = all_diffs
        .iter()
        .filter(|d| d.action != DiffAction::Orphan)
        .count() as u64;

    let _ = tx.send(SyncEvent::Started { total_files, total_bytes });
    ctx.request_repaint();

    // ── Step 1c: 探测目标卷文件系统能力 ─────────────────────────
    let caps: Option<Arc<VolumeCapabilities>> = job
        .folder_pairs
        .iter()
        .find(|p| p.enabled)
        .map(|p| {
            let vol_path = if p.destination.exists() {
                p.destination.clone()
            } else {
                p.destination
                    .parent()
                    .map(|pp| pp.to_path_buf())
                    .unwrap_or_else(|| p.destination.clone())
            };
            Arc::new(detect_volume(&vol_path))
        });

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
    let errors = Arc::new(AtomicU64::new(0));
    let skipped = Arc::new(AtomicU64::new(0));
    let saved_bytes = Arc::new(AtomicU64::new(0));
    let delta_count = Arc::new(AtomicU64::new(0));

    // 收集孤立文件路径（Mirror 模式需要）
    let mut orphan_paths: Vec<PathBuf> = Vec::new();

    let mut task_index: usize = 0;
    let mut handles = Vec::new();

    for d in all_diffs {
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
                let errors2 = errors.clone();
                let saved2 = saved_bytes.clone();
                let delta2 = delta_count.clone();
                let caps2 = caps.clone();
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
                        Ok(Err(e)) => {
                            errors2.fetch_add(1, Ordering::Relaxed);
                            let _ = tx2.send(SyncEvent::FileError {
                                path: d.source.clone(),
                                message: e.to_string(),
                            });
                            crate::log::app_log(
                                &format!("sync copy error: {} — {}", d.source.display(), e),
                                LogLevel::Error,
                            );
                        }
                        Err(e) => {
                            errors2.fetch_add(1, Ordering::Relaxed);
                            let _ = tx2.send(SyncEvent::FileError {
                                path: d.source.clone(),
                                message: format!("task panic: {}", e),
                            });
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

        for path in &orphan_paths {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            match delete_to_trash_or_remove(path) {
                Ok(()) => {
                    deleted.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(SyncEvent::FileDeleted { path: path.clone() });
                    ctx.request_repaint();
                    if let Some(p) = path.parent() {
                        parent_dirs.insert(p.to_path_buf());
                    }
                }
                Err(e) => {
                    let _ = tx.send(SyncEvent::FileError {
                        path: path.clone(),
                        message: format!("删除孤立文件失败: {}", e),
                    });
                }
            }
        }

        // 清理空目录（由深到浅）
        let mut dirs_sorted: Vec<PathBuf> = parent_dirs.into_iter().collect();
        dirs_sorted.sort_by(|a, b| {
            b.components().count().cmp(&a.components().count())
        });
        for dir in dirs_sorted {
            // remove_dir 只能删空目录，非空时静默失败
            let _ = std::fs::remove_dir(&dir);
        }

        // 删除孤立目录（源端不存在的目标端目录，由深到浅）
        if !stop.load(Ordering::Relaxed) {
            for pair in &job.folder_pairs {
                if !pair.enabled { continue; }
                let dirs = collect_orphan_dirs(&pair.source, &pair.destination);
                orphan_dir_count += dirs.len() as u64;
                for dir in dirs {
                    if stop.load(Ordering::Relaxed) { break; }
                    match delete_to_trash_or_remove(&dir) {
                        Ok(()) => {
                            deleted.fetch_add(1, Ordering::Relaxed);
                            let _ = tx.send(SyncEvent::FileDeleted { path: dir });
                            ctx.request_repaint();
                        }
                        Err(e) => {
                            let _ = tx.send(SyncEvent::FileError {
                                path: dir,
                                message: format!("删除孤立目录失败: {}", e),
                            });
                        }
                    }
                }
            }
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
    let final_errors = errors.load(Ordering::Relaxed);
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
        processed_files: final_copied + final_skipped + final_errors,
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

    let _ = tx.send(SyncEvent::Completed { stats, usn_checkpoints });
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

/// 删除文件或目录：优先移入回收站，Shell 不可用时（如安全模式）直接删除。
fn delete_to_trash_or_remove(path: &std::path::Path) -> Result<(), String> {
    match trash::delete(path) {
        Ok(()) => Ok(()),
        Err(_) => {
            if path.is_dir() {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
            } else {
                std::fs::remove_file(path).map_err(|e| e.to_string())
            }
        }
    }
}
