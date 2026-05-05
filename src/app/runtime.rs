use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use eframe::egui;

use crate::app::{flow, PendingDeleteFallback, PendingMassDeleteConfirmation, PendingStartConfirmation};
use crate::engine::events::SyncEvent;
use crate::log::LogLevel;
use crate::model::job::RunTrigger;
use crate::model::preview::PreviewState;
use crate::model::session::{ErrorKind, ErrorScope, SessionStatus, SyncError, SyncSession, WorkerState};

use super::FileSyncApp;

impl FileSyncApp {
    pub fn save_selected_job_with_validation(&mut self) -> bool {
        if let Some(idx) = self.selected_job {
            self.save_job_with_validation(idx)
        } else {
            self.save();
            true
        }
    }

    pub fn start_selected_sync_with_validation(&mut self, ctx: &egui::Context) -> bool {
        let Some(idx) = self.selected_job else {
            return false;
        };
        if let Some(err) = self.validate_folder_pairs_for_start(idx) {
            self.error_message = Some(err);
            return false;
        }
        request_sync_start(self, idx, RunTrigger::Manual, 0, ctx);
        true
    }

    pub fn start_preview_with_validation(&mut self, idx: usize, ctx: &egui::Context) -> bool {
        if let Some(err) = self.validate_folder_pairs_for_start(idx) {
            self.error_message = Some(err);
            return false;
        }
        self.save();
        start_preview(self, ctx);
        true
    }

    pub(super) fn start_sync_entry(
        &mut self,
        idx: usize,
        trigger: RunTrigger,
        retry_attempt: u32,
        ctx: &egui::Context,
    ) {
        start_sync_entry(self, idx, trigger, retry_attempt, ctx);
    }

    pub(super) fn drain_preview(&mut self) {
        drain_preview(self);
    }

    pub(super) fn drain_events(&mut self) {
        drain_events(self);
    }
}

pub(super) fn start_sync_entry(
    app: &mut FileSyncApp,
    idx: usize,
    trigger: RunTrigger,
    retry_attempt: u32,
    ctx: &egui::Context,
) {
    if idx >= app.config.jobs.len() {
        return;
    }

    let job = app.config.jobs[idx].clone();
    let checkpoints = app.job_checkpoints(job.id);
    let concurrency = job.concurrency.max(1);

    let (tx, rx) = flume::bounded(4096);
    let stop = Arc::new(AtomicBool::new(false));

    app.selected_job = Some(idx);
    app.event_rx = Some(rx);
    app.stop_signal = Some(stop.clone());
    app.sync_running = true;
    app.session = Some(SyncSession::new(
        job.id,
        job.name.clone(),
        concurrency,
        trigger,
        retry_attempt,
    ));

    let ctx_clone = ctx.clone();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                let message = format!("failed to create tokio runtime: {}", e);
                crate::log::app_log(&message, LogLevel::Error);
                let _ = tx.send(SyncEvent::StartFailed { message });
                ctx_clone.request_repaint();
                return;
            }
        };
        let interaction = Arc::new(crate::engine::interaction::ChannelSyncInteraction::new(
            trigger,
            tx.clone(),
            ctx_clone.clone(),
        ));
        rt.block_on(crate::engine::executor::run_sync(
            job,
            checkpoints,
            trigger,
            tx,
            ctx_clone,
            stop,
            interaction,
        ));
    });
}

pub(super) fn request_sync_start(
    app: &mut FileSyncApp,
    idx: usize,
    trigger: RunTrigger,
    retry_attempt: u32,
    ctx: &egui::Context,
) {
    if trigger == RunTrigger::Manual && app.requires_risk_confirmation(idx) {
        app.pending_start_confirmation = Some(PendingStartConfirmation {
            job_id: app.config.jobs[idx].id,
            trigger,
            retry_attempt,
        });
        ctx.request_repaint();
        return;
    }
    start_sync_entry(app, idx, trigger, retry_attempt, ctx);
}

pub(super) fn start_preview(app: &mut FileSyncApp, ctx: &egui::Context) {
    let Some(idx) = app.selected_job else { return };
    if idx >= app.config.jobs.len() {
        return;
    }

    let job = app.config.jobs[idx].clone();
    let (tx, rx) = flume::bounded(1);
    app.preview_rx = Some(rx);
    app.preview_state = PreviewState::Loading;
    app.preview_job_is_mirror = job.sync_mode == crate::model::job::SyncMode::Mirror;

    let ctx_clone = ctx.clone();

    std::thread::spawn(move || {
        let result = super::preview_scan::run_preview_scan(job);
        let _ = tx.send(result);
        ctx_clone.request_repaint();
    });
}

fn drain_preview(app: &mut FileSyncApp) {
    let result = match &app.preview_rx {
        Some(rx) => rx.try_recv().ok(),
        None => return,
    };
    if let Some(r) = result {
        app.preview_rx = None;
        app.preview_state = match r {
            Ok(entries) => PreviewState::Ready(entries),
            Err(e) => PreviewState::Error(e),
        };
    }
}

fn drain_events(app: &mut FileSyncApp) {
    const MAX_EVENTS_PER_FRAME: usize = 2000;
    let rx = match app.event_rx.clone() {
        Some(rx) => rx,
        None => return,
    };
    for _ in 0..MAX_EVENTS_PER_FRAME {
        match rx.try_recv() {
            Ok(event) => handle_event(app, event),
            Err(_) => break,
        }
    }
}

fn handle_event(app: &mut FileSyncApp, event: SyncEvent) {
    let Some(mut session) = app.session.take() else { return };

    match event {
        SyncEvent::Started {
            total_files,
            total_bytes,
        } => {
            handle_started(&mut session, total_files, total_bytes);
        }
        SyncEvent::FileStarted {
            worker_id,
            path,
            size,
            is_new,
        } => {
            handle_file_started(&mut session, worker_id, path, size, is_new);
        }
        SyncEvent::FileProgress {
            worker_id,
            bytes_done,
        } => {
            handle_file_progress(&mut session, worker_id, bytes_done);
        }
        SyncEvent::DeleteStarted {
            worker_id,
            path,
            is_dir,
        } => {
            handle_delete_started(&mut session, worker_id, path, is_dir);
        }
        SyncEvent::DeleteFallbackRequired {
            path,
            is_dir,
            message,
            response,
        } => {
            app.pending_delete_fallbacks.push_back(PendingDeleteFallback {
                path,
                is_dir,
                message,
                response,
            });
        }
        SyncEvent::MassDeleteConfirmationRequired { count, response } => {
            app.pending_mass_delete_confirmation =
                Some(PendingMassDeleteConfirmation { count, response });
        }
        SyncEvent::FileCompleted {
            worker_id,
            path,
            size,
            delta,
            saved_bytes,
            ..
        } => {
            handle_file_completed(&mut session, worker_id, path, size, delta, saved_bytes);
        }
        SyncEvent::FileSkipped { .. } => handle_file_skipped(&mut session),
        SyncEvent::FileDeleted { worker_id, path } => {
            handle_file_deleted(&mut session, worker_id, path)
        }
        SyncEvent::WorkerFinished { worker_id } => {
            handle_worker_finished(&mut session, worker_id);
        }
        SyncEvent::FileOrphan { path } => handle_file_orphan(&mut session, path),
        SyncEvent::FileError {
            path,
            message,
            scope,
        } => {
            handle_file_error(&mut session, path, message, scope);
        }
        SyncEvent::Completed {
            stats,
            usn_checkpoints,
            was_stopped,
        } => {
            flow::handle_sync_completed(app, &mut session, stats, usn_checkpoints, was_stopped);
        }
        SyncEvent::StartFailed { message } => {
            flow::handle_start_failed(app, &mut session, message);
        }
        SyncEvent::DiskFull => flow::handle_disk_full(app, &mut session),
        SyncEvent::Paused => session.status = SessionStatus::Paused,
        SyncEvent::Resumed => session.status = SessionStatus::Running,
        SyncEvent::SpeedUpdate { bps: _ } => {}
    }

    app.session = Some(session);
}

fn handle_started(session: &mut SyncSession, total_files: u64, total_bytes: u64) {
    session.stats.total_files = total_files;
    session.stats.total_bytes = total_bytes;
    session.status = SessionStatus::Running;
}

fn handle_file_started(
    session: &mut SyncSession,
    worker_id: usize,
    path: std::path::PathBuf,
    size: u64,
    is_new: bool,
) {
    if worker_id < session.active_workers.len() {
        session.active_workers[worker_id] = WorkerState::Copying {
            path,
            size,
            done: 0,
            is_new,
        };
    }
}

fn handle_file_progress(session: &mut SyncSession, worker_id: usize, bytes_done: u64) {
    if worker_id < session.active_workers.len() {
        if let WorkerState::Copying { done, .. } = &mut session.active_workers[worker_id] {
            *done = bytes_done;
        }
    }
    flow::refresh_speed(session);
}

fn handle_file_completed(
    session: &mut SyncSession,
    worker_id: usize,
    path: std::path::PathBuf,
    size: u64,
    delta: bool,
    saved_bytes: u64,
) {
    if worker_id < session.active_workers.len() {
        session.active_workers[worker_id] = WorkerState::Idle;
    }
    session.stats.copied_files += 1;
    session.stats.processed_files += 1;
    session.stats.copied_bytes += size;
    if delta {
        session.stats.delta_files += 1;
    }
    session.stats.saved_bytes += saved_bytes;
    session
        .copied_log
        .push(crate::model::session::CopiedFileEntry { path, size, delta });
    flow::refresh_speed(session);
}

fn handle_delete_started(
    session: &mut SyncSession,
    worker_id: usize,
    path: std::path::PathBuf,
    is_dir: bool,
) {
    if worker_id < session.active_workers.len() {
        session.active_workers[worker_id] = WorkerState::Deleting { path, is_dir };
    }
}

fn handle_file_skipped(session: &mut SyncSession) {
    session.stats.skipped_files += 1;
    session.stats.processed_files += 1;
}

fn handle_file_deleted(
    session: &mut SyncSession,
    worker_id: usize,
    path: std::path::PathBuf,
) {
    if worker_id < session.active_workers.len() {
        session.active_workers[worker_id] = WorkerState::Idle;
    }
    session.stats.deleted_files += 1;
    session.deleted_paths.push(path);
}

fn handle_worker_finished(session: &mut SyncSession, worker_id: usize) {
    if worker_id < session.active_workers.len() {
        session.active_workers[worker_id] = WorkerState::Idle;
    }
}

fn handle_file_orphan(session: &mut SyncSession, path: std::path::PathBuf) {
    session.orphan_log.push(path);
}

fn handle_file_error(
    session: &mut SyncSession,
    path: std::path::PathBuf,
    message: String,
    scope: ErrorScope,
) {
    session.stats.error_count += 1;
    match scope {
        ErrorScope::Scan => session.stats.scan_error_count += 1,
        ErrorScope::Copy => {
            session.stats.copy_error_count += 1;
            session.stats.processed_files += 1;
        }
        ErrorScope::Delete => session.stats.delete_error_count += 1,
    }
    session.errors.push(SyncError {
        timestamp: chrono::Utc::now(),
        path,
        scope,
        kind: ErrorKind::IoError,
        message,
    });
}
