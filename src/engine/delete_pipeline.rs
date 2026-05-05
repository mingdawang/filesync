use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use egui::Context;
use flume::Sender;
use tokio::sync::Semaphore;

use crate::engine::delete::{delete_failed_message, delete_with_mode, DeleteOutcome};
use crate::engine::events::SyncEvent;
use crate::engine::interaction::SyncInteraction;
use crate::engine::messages;
use crate::engine::scan_plan::collect_orphan_dirs;
use crate::log::LogLevel;
use crate::model::job::{SyncJob, SyncMode};
use crate::model::session::ErrorScope;

pub(crate) struct DeletePipelineResult {
    pub(crate) deleted: u64,
    pub(crate) delete_errors: u64,
    pub(crate) orphan_dir_count: u64,
    pub(crate) threshold_blocked: bool,
}

pub(crate) async fn run_delete_pipeline(
    job: &SyncJob,
    orphan_paths: &[PathBuf],
    tx: &Sender<SyncEvent>,
    _ctx: &Context,
    stop: &Arc<AtomicBool>,
    interaction: Arc<dyn SyncInteraction>,
) -> DeletePipelineResult {
    let mut orphan_dir_count = 0;

    if job.sync_mode != SyncMode::Mirror || stop.load(Ordering::Relaxed) {
        if job.sync_mode != SyncMode::Mirror && !stop.load(Ordering::Relaxed) {
            for pair in &job.folder_pairs {
                if !pair.enabled {
                    continue;
                }
                for dir in collect_orphan_dirs(&pair.source, &pair.destination) {
                    orphan_dir_count += 1;
                    let _ = tx.send(SyncEvent::FileOrphan { path: dir });
                }
            }
        }
        return DeletePipelineResult {
            deleted: 0,
            delete_errors: 0,
            orphan_dir_count,
            threshold_blocked: false,
        };
    }

    let (delete_targets, parent_dirs, counted_orphan_dirs) = build_delete_targets(job, orphan_paths);
    orphan_dir_count = counted_orphan_dirs;
    let delete_count = delete_targets.len() as u64;

    if mass_delete_block_reason(delete_count, job.schedule.delete_threshold, interaction.as_ref()) {
        let message = if interaction.allows_prompts() {
            messages::mirror_delete_cancelled(delete_count, job.schedule.delete_threshold)
        } else {
            messages::scheduled_mirror_delete_blocked(delete_count, job.schedule.delete_threshold)
        };
        let _ = tx.send(SyncEvent::FileError {
            path: PathBuf::from("<mirror-delete-threshold>"),
            message,
            scope: ErrorScope::Delete,
        });
        return DeletePipelineResult {
            deleted: 0,
            delete_errors: 0,
            orphan_dir_count,
            threshold_blocked: true,
        };
    }

    let deleted = Arc::new(AtomicU64::new(0));
    let delete_errors = Arc::new(AtomicU64::new(0));
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
        let stop_delete = stop.clone();
        let stop_for_delete = stop_delete.clone();
        let deleted_count = deleted.clone();
        let error_count = delete_errors.clone();
        let delete_mode = job.delete_mode.clone();
        let delete_fallback_policy = job.delete_fallback_policy.clone();
        let interaction = interaction.clone();

        let handle = tokio::spawn(async move {
            let _permit = permit;
            let _ = tx_delete.send(SyncEvent::DeleteStarted {
                worker_id,
                path: path.clone(),
                is_dir,
            });

            let delete_path = path.clone();
            let result = tokio::task::spawn_blocking(move || {
                delete_with_mode(
                    &delete_path,
                    &delete_mode,
                    &delete_fallback_policy,
                    interaction.as_ref(),
                    is_dir,
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
                Ok(Err(_)) if stop_delete.load(Ordering::Relaxed) => {
                    let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                }
                Ok(Err(err)) => {
                    error_count.fetch_add(1, Ordering::Relaxed);
                    let _ = tx_delete.send(SyncEvent::FileError {
                        path: path.clone(),
                        message: delete_failed_message(is_dir, &err),
                        scope: ErrorScope::Delete,
                    });
                    let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                }
                Err(_) if stop_delete.load(Ordering::Relaxed) => {
                    let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                }
                Err(err) => {
                    error_count.fetch_add(1, Ordering::Relaxed);
                    let _ = tx_delete.send(SyncEvent::FileError {
                        path: path.clone(),
                        message: messages::delete_task_panic(&err.to_string()),
                        scope: ErrorScope::Delete,
                    });
                    let _ = tx_delete.send(SyncEvent::WorkerFinished { worker_id });
                    crate::log::app_log(
                        &format!("delete task panic: {} - {}", path.display(), err),
                        LogLevel::Error,
                    );
                }
            }
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

    DeletePipelineResult {
        deleted: deleted.load(Ordering::Relaxed),
        delete_errors: delete_errors.load(Ordering::Relaxed),
        orphan_dir_count,
        threshold_blocked: false,
    }
}

fn build_delete_targets(
    job: &SyncJob,
    orphan_paths: &[PathBuf],
) -> (Vec<(PathBuf, bool)>, HashSet<PathBuf>, u64) {
    let mut parent_dirs = HashSet::new();
    let mut delete_targets = Vec::new();
    let mut orphan_dir_count = 0;

    for path in orphan_paths {
        if let Some(parent) = path.parent() {
            parent_dirs.insert(parent.to_path_buf());
        }
        delete_targets.push((path.clone(), false));
    }

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

    (delete_targets, parent_dirs, orphan_dir_count)
}

fn mass_delete_block_reason(
    delete_count: u64,
    threshold: u64,
    interaction: &dyn SyncInteraction,
) -> bool {
    if delete_count <= threshold {
        return false;
    }

    if interaction.allows_prompts() {
        !interaction.confirm_mass_delete(delete_count)
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::mass_delete_block_reason;
    use crate::engine::events::DeleteFallbackChoice;
    use crate::engine::interaction::SyncInteraction;
    use std::path::Path;

    struct MockInteraction {
        prompts: bool,
        confirm: bool,
    }

    impl SyncInteraction for MockInteraction {
        fn allows_prompts(&self) -> bool {
            self.prompts
        }

        fn confirm_mass_delete(&self, _count: u64) -> bool {
            self.confirm
        }

        fn request_delete_fallback(
            &self,
            _path: &Path,
            _is_dir: bool,
            _message: String,
        ) -> DeleteFallbackChoice {
            DeleteFallbackChoice::Skip
        }
    }

    #[test]
    fn unattended_mass_delete_is_blocked() {
        let interaction = MockInteraction {
            prompts: false,
            confirm: true,
        };
        assert!(mass_delete_block_reason(12, 5, &interaction));
    }

    #[test]
    fn attended_mass_delete_can_be_cancelled() {
        let interaction = MockInteraction {
            prompts: true,
            confirm: false,
        };
        assert!(mass_delete_block_reason(12, 5, &interaction));
    }

    #[test]
    fn under_threshold_does_not_block() {
        let interaction = MockInteraction {
            prompts: false,
            confirm: false,
        };
        assert!(!mass_delete_block_reason(5, 5, &interaction));
    }
}
