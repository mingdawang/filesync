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
    let delete_result = run_delete_stage(&job, &copy_result.orphan_paths, &tx, &ctx, &stop, interaction).await;

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

    run_delete_pipeline(job, orphan_paths, tx, ctx, stop, interaction).await
}
