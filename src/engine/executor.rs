use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use egui::Context;
use flume::Sender;

use crate::engine::copy_pipeline::{run_copy_pipeline, spawn_speed_reporter};
use crate::engine::delete_pipeline::run_delete_pipeline;
use crate::engine::events::SyncEvent;
use crate::engine::interaction::SyncInteraction;
use crate::engine::scan_plan::{build_sync_plan, count_non_orphan_files};
use crate::model::job::{RunTrigger, SyncJob, SyncMode, UsnCheckpoint};
use crate::model::session::SyncStats;

pub async fn run_sync(
    job: SyncJob,
    checkpoints: HashMap<String, UsnCheckpoint>,
    _trigger: RunTrigger,
    tx: Sender<SyncEvent>,
    ctx: Context,
    stop: Arc<AtomicBool>,
    interaction: Arc<dyn SyncInteraction>,
) {
    let plan = build_sync_plan(&job, &checkpoints, &tx, &ctx).await;
    let total_files = count_non_orphan_files(&plan.diffs);

    let _ = tx.send(SyncEvent::Started {
        total_files,
        total_bytes: plan.total_bytes,
    });
    ctx.request_repaint();

    let copy_result = run_copy_stage(&job, plan.diffs, &tx, &ctx, &stop).await;
    let delete_result = run_delete_stage(
        &job,
        &copy_result.orphan_paths,
        &plan.orphan_directories,
        &tx,
        &ctx,
        &stop,
        interaction,
    )
    .await;

    let was_stopped = stop.load(Ordering::Relaxed);
    let final_delete_errors =
        delete_result.delete_errors + u64::from(delete_result.threshold_blocked);
    let final_errors = plan.scan_error_count + copy_result.copy_errors + final_delete_errors;

    let stats = SyncStats {
        total_files,
        total_bytes: plan.total_bytes,
        copied_files: copy_result.copied,
        copied_bytes: copy_result.bytes_transferred.load(Ordering::Relaxed),
        skipped_files: copy_result.skipped,
        error_count: final_errors,
        scan_error_count: plan.scan_error_count,
        copy_error_count: copy_result.copy_errors,
        delete_error_count: final_delete_errors,
        processed_files: copy_result.copied + copy_result.skipped + copy_result.copy_errors,
        delta_files: copy_result.delta_count,
        saved_bytes: copy_result.saved_bytes,
        deleted_files: delete_result.deleted,
        orphan_files: copy_result.orphan_paths.len() as u64 + delete_result.orphan_dir_count,
        speed_bps: 0,
    };

    let usn_checkpoints = if was_stopped {
        HashMap::new()
    } else {
        plan.new_checkpoints
    };

    let _ = tx.send(SyncEvent::Completed {
        stats,
        usn_checkpoints,
        was_stopped,
    });
    ctx.request_repaint();
}

async fn run_copy_stage(
    job: &SyncJob,
    diffs: Vec<crate::engine::scan_plan::PlannedDiff>,
    tx: &Sender<SyncEvent>,
    ctx: &Context,
    stop: &Arc<AtomicBool>,
) -> crate::engine::copy_pipeline::CopyPipelineResult {
    let bytes_transferred = Arc::new(std::sync::atomic::AtomicU64::new(0));
    spawn_speed_reporter(bytes_transferred.clone(), tx.clone(), ctx.clone(), stop.clone());

    run_copy_pipeline(job, diffs, tx, ctx, stop, bytes_transferred).await
}

async fn run_delete_stage(
    job: &SyncJob,
    orphan_paths: &[std::path::PathBuf],
    orphan_directories: &[std::path::PathBuf],
    tx: &Sender<SyncEvent>,
    ctx: &Context,
    stop: &Arc<AtomicBool>,
    interaction: Arc<dyn SyncInteraction>,
) -> crate::engine::delete_pipeline::DeletePipelineResult {
    if job.sync_mode != SyncMode::Mirror && stop.load(Ordering::Relaxed) {
        return crate::engine::delete_pipeline::DeletePipelineResult {
            deleted: 0,
            delete_errors: 0,
            orphan_dir_count: 0,
            threshold_blocked: false,
        };
    }

    run_delete_pipeline(job, orphan_paths, orphan_directories, tx, ctx, stop, interaction).await
}

#[cfg(test)]
mod tests {
    use super::run_sync;
    use crate::engine::events::{DeleteFallbackChoice, SyncEvent};
    use crate::engine::interaction::SyncInteraction;
    use crate::model::config::CompareMethod;
    use crate::model::job::{
        DeleteFallbackPolicy, DeleteMode, FolderPair, RunTrigger, SyncJob, SyncMode,
    };
    use filetime::{set_file_mtime, FileTime};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    struct MockInteraction {
        prompts: bool,
        confirm_mass_delete: bool,
        fallback_choice: DeleteFallbackChoice,
    }

    impl SyncInteraction for MockInteraction {
        fn allows_prompts(&self) -> bool {
            self.prompts
        }

        fn confirm_mass_delete(&self, _count: u64) -> bool {
            self.confirm_mass_delete
        }

        fn request_delete_fallback(
            &self,
            _path: &Path,
            _is_dir: bool,
            _message: String,
        ) -> DeleteFallbackChoice {
            self.fallback_choice
        }
    }

    #[test]
    fn hash_compare_can_downgrade_update_to_skip() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let src_file = src.path().join("same.txt");
        let dst_file = dst.path().join("same.txt");

        std::fs::write(&src_file, b"same-content").unwrap();
        std::fs::write(&dst_file, b"same-content").unwrap();
        let newer = FileTime::from_system_time(SystemTime::now() + Duration::from_secs(5));
        set_file_mtime(&src_file, newer).unwrap();

        let mut job = single_pair_job(src.path(), dst.path());
        job.compare_method = CompareMethod::Hash;

        let events = run_job_and_collect(job, RunTrigger::Manual, Arc::new(MockInteraction {
            prompts: true,
            confirm_mass_delete: true,
            fallback_choice: DeleteFallbackChoice::Skip,
        }), false);

        let completed = completed_event(&events);
        assert_eq!(completed.0.copied_files, 0);
        assert_eq!(completed.0.skipped_files, 1);
        assert_eq!(completed.0.error_count, 0);
    }

    #[test]
    fn mirror_mode_deletes_orphans_and_empty_dirs() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        std::fs::write(src.path().join("keep.txt"), b"keep").unwrap();
        std::fs::write(dst.path().join("orphan.txt"), b"remove").unwrap();
        std::fs::create_dir_all(dst.path().join("orphan_dir\\nested")).unwrap();

        let mut job = single_pair_job(src.path(), dst.path());
        job.sync_mode = SyncMode::Mirror;
        job.delete_mode = DeleteMode::Direct;
        job.delete_fallback_policy = DeleteFallbackPolicy::Fail;

        let events = run_job_and_collect(job, RunTrigger::Manual, Arc::new(MockInteraction {
            prompts: true,
            confirm_mass_delete: true,
            fallback_choice: DeleteFallbackChoice::Skip,
        }), false);

        let completed = completed_event(&events);
        assert_eq!(completed.0.error_count, 0);
        assert_eq!(completed.0.deleted_files, 3);
        assert!(!dst.path().join("orphan.txt").exists());
        assert!(!dst.path().join("orphan_dir").exists());
    }

    #[test]
    fn stopped_run_reports_empty_usn_checkpoints() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("file.txt"), b"content").unwrap();

        let job = single_pair_job(src.path(), dst.path());
        let events = run_job_and_collect(job, RunTrigger::Manual, Arc::new(MockInteraction {
            prompts: true,
            confirm_mass_delete: true,
            fallback_choice: DeleteFallbackChoice::Skip,
        }), true);

        let completed = completed_event(&events);
        assert!(completed.2);
        assert!(completed.1.is_empty());
    }

    fn single_pair_job(src: &Path, dst: &Path) -> SyncJob {
        let mut job = SyncJob::new("job".into(), 2);
        let mut pair = FolderPair::new();
        pair.source = src.to_path_buf();
        pair.destination = dst.to_path_buf();
        job.folder_pairs = vec![pair];
        job
    }

    fn run_job_and_collect(
        job: SyncJob,
        trigger: RunTrigger,
        interaction: Arc<dyn SyncInteraction>,
        stopped: bool,
    ) -> Vec<SyncEvent> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, rx) = flume::unbounded();
        let ctx = egui::Context::default();
        let stop = Arc::new(AtomicBool::new(stopped));

        rt.block_on(run_sync(
            job,
            HashMap::new(),
            trigger,
            tx,
            ctx,
            stop,
            interaction,
        ));

        rx.drain().collect()
    }

    fn completed_event(
        events: &[SyncEvent],
    ) -> (&crate::model::session::SyncStats, &HashMap<String, (u64, i64)>, bool) {
        events
            .iter()
            .find_map(|event| match event {
                SyncEvent::Completed {
                    stats,
                    usn_checkpoints,
                    was_stopped,
                } => Some((stats, usn_checkpoints, *was_stopped)),
                _ => None,
            })
            .unwrap()
    }
}
