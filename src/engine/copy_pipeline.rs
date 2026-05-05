use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::bail;
use egui::Context;
use flume::Sender;
use tokio::sync::Semaphore;

use crate::engine::diff::DiffAction;
use crate::engine::events::SyncEvent;
use crate::engine::messages;
use crate::engine::scan_plan::PlannedDiff;
use crate::engine::copier;
use crate::log::LogLevel;
use crate::model::job::{SyncJob, SyncMode};
use crate::model::session::ErrorScope;

pub(crate) struct CopyPipelineResult {
    pub(crate) copied: u64,
    pub(crate) copy_errors: u64,
    pub(crate) skipped: u64,
    pub(crate) saved_bytes: u64,
    pub(crate) delta_count: u64,
    pub(crate) bytes_transferred: Arc<AtomicU64>,
    pub(crate) orphan_paths: Vec<PathBuf>,
}

pub(crate) fn spawn_speed_reporter(
    bytes_transferred: Arc<AtomicU64>,
    tx: Sender<SyncEvent>,
    _ctx: Context,
    stop: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut previous = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let current = bytes_transferred.load(Ordering::Relaxed);
            let bps = current.saturating_sub(previous);
            previous = current;
            let _ = tx.send(SyncEvent::SpeedUpdate { bps });
        }
    });
}

pub(crate) async fn run_copy_pipeline(
    job: &SyncJob,
    diffs: Vec<PlannedDiff>,
    tx: &Sender<SyncEvent>,
    _ctx: &Context,
    stop: &Arc<AtomicBool>,
    bytes_transferred: Arc<AtomicU64>,
) -> CopyPipelineResult {
    let delta_threshold = job.engine_options.delta_threshold_mb * 1024 * 1024;
    let unbuffered_threshold = job.engine_options.unbuffered_threshold_mb * 1024 * 1024;
    let verify_after_copy = job.engine_options.verify_after_copy;
    let concurrency = job.concurrency.max(1);

    let sem = Arc::new(Semaphore::new(concurrency));
    let copied = Arc::new(AtomicU64::new(0));
    let copy_errors = Arc::new(AtomicU64::new(0));
    let skipped = Arc::new(AtomicU64::new(0));
    let saved_bytes = Arc::new(AtomicU64::new(0));
    let delta_count = Arc::new(AtomicU64::new(0));
    let mut orphan_paths = Vec::new();
    let mut handles = Vec::new();
    let mut task_index = 0usize;

    for planned in diffs {
        let diff = planned.diff;
        if stop.load(Ordering::Relaxed) {
            break;
        }

        match diff.action {
            DiffAction::Orphan => {
                orphan_paths.push(diff.destination.clone());
                if job.sync_mode != SyncMode::Mirror {
                    let _ = tx.send(SyncEvent::FileOrphan {
                        path: diff.destination.clone(),
                    });
                }
            }
            DiffAction::Skip => {
                skipped.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(SyncEvent::FileSkipped {
                    path: diff.relative_path,
                });
            }
            DiffAction::Create | DiffAction::Update => {
                let worker_id = task_index % concurrency;
                task_index += 1;

                let permit = match sem.clone().acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => {
                        crate::log::app_log(
                            "semaphore closed unexpectedly, aborting copy loop",
                            LogLevel::Error,
                        );
                        break;
                    }
                };

                let tx_copy = tx.clone();
                let stop_copy = stop.clone();
                let copied_count = copied.clone();
                let error_count = copy_errors.clone();
                let saved_count = saved_bytes.clone();
                let delta_count_ref = delta_count.clone();
                let caps = planned.caps.clone();
                let transferred = bytes_transferred.clone();
                let size = diff.size;
                let use_delta = delta_threshold > 0 && size >= delta_threshold;

                let handle = tokio::spawn(async move {
                    let _permit = permit;
                    let _ = tx_copy.send(SyncEvent::FileStarted {
                        worker_id,
                        path: diff.source.clone(),
                        size,
                        is_new: diff.action == DiffAction::Create,
                    });

                    let src = diff.source.clone();
                    let dst = diff.destination.clone();
                    let tx_progress = tx_copy.clone();
                    let stop_progress = stop_copy.clone();

                    let result = tokio::task::spawn_blocking(move || {
                        if use_delta && dst.exists() {
                            match crate::engine::delta::delta_sync(
                                &src,
                                &dst,
                                worker_id,
                                &tx_progress,
                                &stop_progress,
                            ) {
                                Ok((_, saved)) => {
                                    saved_count.fetch_add(saved, Ordering::Relaxed);
                                    delta_count_ref.fetch_add(1, Ordering::Relaxed);
                                    Ok((true, saved))
                                }
                                Err(_) if stop_progress.load(Ordering::Relaxed) => bail!("stopped"),
                                Err(_) => copier::copy_file_with_caps(
                                    &src,
                                    &dst,
                                    worker_id,
                                    size,
                                    &tx_progress,
                                    &stop_progress,
                                    caps.as_deref(),
                                    verify_after_copy,
                                    unbuffered_threshold,
                                )
                                .map(|_| (false, 0)),
                            }
                        } else {
                            copier::copy_file_with_caps(
                                &src,
                                &dst,
                                worker_id,
                                size,
                                &tx_progress,
                                &stop_progress,
                                caps.as_deref(),
                                verify_after_copy,
                                unbuffered_threshold,
                            )
                            .map(|_| (false, 0))
                        }
                    })
                    .await;

                    match result {
                        Ok(Ok((delta_used, saved))) => {
                            copied_count.fetch_add(1, Ordering::Relaxed);
                            transferred.fetch_add(size, Ordering::Relaxed);
                            let _ = tx_copy.send(SyncEvent::FileCompleted {
                                worker_id,
                                path: diff.relative_path,
                                size,
                                delta: delta_used,
                                saved_bytes: saved,
                            });
                        }
                        Ok(Err(_)) if stop_copy.load(Ordering::Relaxed) => {}
                        Ok(Err(err)) => {
                            error_count.fetch_add(1, Ordering::Relaxed);
                            let _ = tx_copy.send(SyncEvent::FileError {
                                path: diff.source.clone(),
                                message: err.to_string(),
                                scope: ErrorScope::Copy,
                            });
                            let _ = tx_copy.send(SyncEvent::WorkerFinished { worker_id });
                            crate::log::app_log(
                                &format!("sync copy error: {} - {}", diff.source.display(), err),
                                LogLevel::Error,
                            );
                        }
                        Err(_) if stop_copy.load(Ordering::Relaxed) => {}
                        Err(err) => {
                            error_count.fetch_add(1, Ordering::Relaxed);
                            let _ = tx_copy.send(SyncEvent::FileError {
                                path: diff.source.clone(),
                                message: messages::copy_task_panic(&err.to_string()),
                                scope: ErrorScope::Copy,
                            });
                            let _ = tx_copy.send(SyncEvent::WorkerFinished { worker_id });
                            crate::log::app_log(
                                &format!("sync task panic: {} - {}", diff.source.display(), err),
                                LogLevel::Error,
                            );
                        }
                    }
                });

                handles.push(handle);
            }
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    CopyPipelineResult {
        copied: copied.load(Ordering::Relaxed),
        copy_errors: copy_errors.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
        saved_bytes: saved_bytes.load(Ordering::Relaxed),
        delta_count: delta_count.load(Ordering::Relaxed),
        bytes_transferred,
        orphan_paths,
    }
}
