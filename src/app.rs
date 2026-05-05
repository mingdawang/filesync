use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::storage;
use crate::engine::events::{DeleteFallbackChoice, SyncEvent};
use crate::i18n::{is_zh, t};
use crate::model::config::AppConfig;
use crate::model::job::RunTrigger;
use crate::model::preview::{PreviewEntry, PreviewState};
use crate::model::runtime::{JobStateRecord, JobTransientState};
use crate::model::session::{ErrorKind, ErrorScope, SessionStatus, SyncError, SyncSession, WorkerState};
use crate::log::LogLevel;

mod dialogs;
mod flow;
mod schedule;
mod shell;
mod strings;
mod support;

use self::support::{run_preview_scan, setup_fonts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Success,
    Warning,
}

pub struct AppNotification {
    pub title: String,
    pub body: String,
    pub created_at: std::time::Instant,
    pub kind: NotificationKind,
}

struct PendingDeleteFallback {
    path: std::path::PathBuf,
    is_dir: bool,
    message: String,
    response: std::sync::mpsc::Sender<DeleteFallbackChoice>,
}

struct PendingMassDeleteConfirmation {
    count: u64,
    response: std::sync::mpsc::Sender<bool>,
}

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub job_id: uuid::Uuid,
    pub trigger: RunTrigger,
    pub retry_attempt: u32,
    pub ready_at: DateTime<Utc>,
}

struct PendingStartConfirmation {
    job_id: uuid::Uuid,
    trigger: RunTrigger,
    retry_attempt: u32,
}

enum CloseDialogAction {
    Minimize,
    Quit,
    Cancel,
}

pub struct FileSyncApp {
    pub config: AppConfig,
    /// 当前选中的任务索引
    pub selected_job: Option<usize>,
    /// 设置有未保存的修改
    settings_dirty: bool,
    /// 当前同步会话
    pub session: Option<SyncSession>,
    /// 是否正在同步（控制按钮状态）
    pub sync_running: bool,
    /// 等待删除确认的任务索引
    pub pending_delete: Option<usize>,
    /// 排除规则输入框内容
    pub new_exclusion_input: String,
    /// 排除规则 Glob 验证错误
    pub exclusion_error: Option<String>,
    /// 全局错误弹窗消息
    pub error_message: Option<String>,
    /// 设置窗口是否打开
    pub settings_open: bool,
    /// 关于窗口是否打开
    pub about_open: bool,
    /// 差异预览状态
    pub preview_state: PreviewState,
    /// 预览扫描时对应任务的同步模式（Mirror = true）
    pub preview_job_is_mirror: bool,
    /// 同步引擎事件接收端
    event_rx: Option<flume::Receiver<SyncEvent>>,
    /// 预览结果接收端
    preview_rx: Option<flume::Receiver<Result<Vec<PreviewEntry>, String>>>,
    /// 任务运行队列（顺序执行多个任务）
    pub job_queue: std::collections::VecDeque<QueueEntry>,
    job_transient: HashMap<uuid::Uuid, JobTransientState>,
    /// 下一帧启动队列中的任务
    pending_queue_start: bool,
    /// 停止信号
    pub stop_signal: Option<Arc<AtomicBool>>,
    /// 应用内完成通知
    pub notification: Option<AppNotification>,
    /// 系统托盘图标（持有以保持图标存活）
    tray: Option<crate::tray::AppTray>,
    /// 是否正在显示关闭确认对话框
    close_dialog_open: bool,
    /// 关闭对话框中的"不再询问"勾选状态
    close_dialog_remember: bool,
    /// 是否正在显示"未保存修改"确认对话框
    unsaved_dialog_open: bool,
    /// 底部进度面板的当前高度，避免刷新后回到默认值
    progress_panel_height: Option<f32>,
    pending_delete_fallbacks: std::collections::VecDeque<PendingDeleteFallback>,
    pending_mass_delete_confirmation: Option<PendingMassDeleteConfirmation>,
    pending_start_confirmation: Option<PendingStartConfirmation>,
    history_open: bool,
}

impl FileSyncApp {
    pub fn new(cc: &eframe::CreationContext<'_>, tray: Option<crate::tray::AppTray>) -> Self {
        crate::log::app_log("FileSyncApp::new starting", LogLevel::Info);

        setup_fonts(&cc.egui_ctx);
        crate::log::app_log("font setup complete", LogLevel::Info);

        // 保存 egui 上下文供 wndproc 钩子在拦截 SC_CLOSE 后调用 request_repaint()
        crate::tray::set_egui_ctx(cc.egui_ctx.clone());

        let config = storage::load().unwrap_or_else(|e| {
            crate::log::app_log(&format!("failed to load config, using defaults: {}", e), LogLevel::Error);
            AppConfig::default()
        });
        let mut job_transient = HashMap::new();
        for job in &config.jobs {
            job_transient.insert(
                job.id,
                JobTransientState {
                    dirty: false,
                    ..JobTransientState::default()
                },
            );
        }

        // 若托盘可用，启动后台事件转发线程（Show/Quit 均通过 Win32 直接处理）
        if let Some(ref t) = tray {
            crate::log::app_log("starting tray event relay", LogLevel::Info);
            t.start_event_relay();
        } else {
            crate::log::app_log("system tray not available", LogLevel::Info);
        }

        crate::log::app_log("FileSyncApp::new complete", LogLevel::Info);
        Self {
            config,
            selected_job: None,
            settings_dirty: false,
            session: None,
            sync_running: false,
            pending_delete: None,
            new_exclusion_input: String::new(),
            exclusion_error: None,
            error_message: None,
            settings_open: false,
            about_open: false,
            preview_state: PreviewState::Idle,
            preview_job_is_mirror: false,
            event_rx: None,
            preview_rx: None,
            job_queue: std::collections::VecDeque::new(),
            job_transient,
            pending_queue_start: false,
            stop_signal: None,
            notification: None,
            tray,
            close_dialog_open: false,
            close_dialog_remember: false,
            unsaved_dialog_open: false,
            progress_panel_height: None,
            pending_delete_fallbacks: std::collections::VecDeque::new(),
            pending_mass_delete_confirmation: None,
            pending_start_confirmation: None,
            history_open: false,
        }
    }

    /// 停止同步（如正在运行）、释放托盘，然后退出进程。
    /// 如果有未保存的修改，先弹出确认对话框。
    fn quit_app(&mut self) {
        crate::log::app_log("quit_app() called", LogLevel::Info);
        if self.is_dirty() {
            self.unsaved_dialog_open = true;
            return;
        }
        self.quit_app_now();
    }

    /// 实际退出：停止同步、释放托盘、退出进程。
    fn quit_app_now(&mut self) {
        crate::log::app_log("quit_app_now() called, exiting process", LogLevel::Info);
        if self.sync_running {
            self.stop_sync();
        }
        self.tray = None;
        std::process::exit(0);
    }

    /// 任一 job 或 settings 有未保存的修改
    pub fn is_dirty(&self) -> bool {
        self.settings_dirty || self.job_transient.values().any(|state| state.dirty)
    }

    /// 当前选中 job 是否有未保存的修改
    pub fn current_job_dirty(&self) -> bool {
        self.selected_job
            .map(|idx| {
                self.config
                    .jobs
                    .get(idx)
                    .and_then(|job| self.job_transient.get(&job.id))
                    .map(|state| state.dirty)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub fn job_state(&self, job_id: uuid::Uuid) -> Option<&JobStateRecord> {
        self.config.job_states.iter().find(|state| state.job_id == job_id)
    }

    pub fn job_state_mut(&mut self, job_id: uuid::Uuid) -> Option<&mut JobStateRecord> {
        self.config
            .job_states
            .iter_mut()
            .find(|state| state.job_id == job_id)
    }

    pub fn ensure_job_state_mut(&mut self, job_id: uuid::Uuid) -> &mut JobStateRecord {
        if let Some(idx) = self.config.job_states.iter().position(|state| state.job_id == job_id) {
            return &mut self.config.job_states[idx];
        }
        self.config.job_states.push(JobStateRecord {
            job_id,
            ..JobStateRecord::default()
        });
        self.config.job_states.last_mut().unwrap()
    }

    pub fn mark_job_dirty(&mut self, job_id: uuid::Uuid) {
        self.job_transient.entry(job_id).or_default().dirty = true;
    }

    pub fn clear_job_dirty(&mut self, job_id: uuid::Uuid) {
        self.job_transient.entry(job_id).or_default().dirty = false;
    }

    pub fn job_checkpoints(&self, job_id: uuid::Uuid) -> HashMap<String, crate::model::job::UsnCheckpoint> {
        self.job_transient
            .get(&job_id)
            .map(|state| state.last_sync_checkpoints.clone())
            .unwrap_or_default()
    }


    /// 检查任务 `idx` 的文件夹对是否存在部分配置（已启用但只填了源或目标之一）。
    /// 返回 `None` 表示通过；返回 `Some(error_msg)` 表示有问题。
    /// 用于保存校验——只要没有不完整的对就允许保存（无已启用对也可保存）。
    pub fn validate_folder_pairs_for_save(&self, idx: usize) -> Option<String> {
        if self.job_has_partial_enabled_folder_pair(idx) {
            Some(
                t(
                    "存在已启用但源/目标路径不完整的文件夹对，请检查配置后再保存。",
                    "Some enabled folder pairs have incomplete paths. Please fix them before saving.",
                )
                .into(),
            )
        } else {
            None
        }
    }

    /// 检查任务 `idx` 是否可以启动操作（预览 / 同步）：
    /// - 无部分配置（已启用对必须同时填写源和目标）
    /// - 至少存在一个同时填写了源和目标的已启用对
    /// 返回 `None` 表示通过；返回 `Some(error_msg)` 表示有问题。
    pub fn validate_folder_pairs_for_start(&self, idx: usize) -> Option<String> {
        if self.job_has_partial_enabled_folder_pair(idx) {
            return Some(
                t(
                    "存在已启用但源/目标路径不完整的文件夹对，请检查配置。",
                    "Some enabled folder pairs have incomplete paths. Please fix them.",
                )
                .into(),
            );
        }
        if !self.job_has_valid_enabled_folder_pair(idx) {
            Some(
                t(
                    "请先配置至少一个已启用且源/目标路径均已填写的文件夹对。",
                    "Please configure at least one enabled folder pair with source and destination paths.",
                )
                .into(),
            )
        } else {
            None
        }
    }

    pub fn job_has_partial_enabled_folder_pair(&self, idx: usize) -> bool {
        self.config
            .jobs
            .get(idx)
            .map_or(false, |job| has_partial_enabled_folder_pair(&job.folder_pairs))
    }

    pub fn job_has_valid_enabled_folder_pair(&self, idx: usize) -> bool {
        self.config
            .jobs
            .get(idx)
            .map_or(false, |job| has_valid_enabled_folder_pair(&job.folder_pairs))
    }

    /// 保存配置到磁盘
    pub fn save(&mut self) {
        match storage::save(&self.config) {
            Ok(()) => {
                self.settings_dirty = false;
                let job_ids: Vec<_> = self.config.jobs.iter().map(|job| job.id).collect();
                for job_id in job_ids {
                    self.clear_job_dirty(job_id);
                }
            }
            Err(e) => {
                self.error_message = Some(if is_zh() {
                    format!("保存失败: {}", e)
                } else {
                    format!("Save failed: {}", e)
                });
            }
        }
    }

    pub fn save_job_with_validation(&mut self, idx: usize) -> bool {
        if let Some(err) = self.validate_folder_pairs_for_save(idx) {
            self.error_message = Some(err);
            return false;
        }
        self.save();
        true
    }

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
        self.request_sync_start(idx, RunTrigger::Manual, 0, ctx);
        true
    }

    pub fn start_preview_with_validation(
        &mut self,
        idx: usize,
        ctx: &egui::Context,
    ) -> bool {
        if let Some(err) = self.validate_folder_pairs_for_start(idx) {
            self.error_message = Some(err);
            return false;
        }
        self.save();
        self.start_preview(ctx);
        true
    }

    /// 启动同步任务
    fn start_sync_entry(
        &mut self,
        idx: usize,
        trigger: RunTrigger,
        retry_attempt: u32,
        ctx: &egui::Context,
    ) {
        if idx >= self.config.jobs.len() {
            return;
        }

        let job = self.config.jobs[idx].clone();
        let checkpoints = self.job_checkpoints(job.id);
        let concurrency = job.concurrency.max(1);

        let (tx, rx) = flume::bounded(4096);
        let stop = Arc::new(AtomicBool::new(false));

        self.selected_job = Some(idx);
        self.event_rx = Some(rx);
        self.stop_signal = Some(stop.clone());
        self.sync_running = true;
        self.session = Some(SyncSession::new(
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
                    crate::log::app_log(
                        &message,
                        LogLevel::Error,
                    );
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

    /// 启动后台预览扫描
    pub fn start_preview(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.selected_job else { return };
        if idx >= self.config.jobs.len() {
            return;
        }

        let job = self.config.jobs[idx].clone();
        let (tx, rx) = flume::bounded(1);
        self.preview_rx = Some(rx);
        self.preview_state = PreviewState::Loading;
        self.preview_job_is_mirror = job.sync_mode == crate::model::job::SyncMode::Mirror;

        let ctx_clone = ctx.clone();

        std::thread::spawn(move || {
            let result = run_preview_scan(job);
            let _ = tx.send(result);
            ctx_clone.request_repaint();
        });
    }

    fn request_sync_start(
        &mut self,
        idx: usize,
        trigger: RunTrigger,
        retry_attempt: u32,
        ctx: &egui::Context,
    ) {
        if trigger == RunTrigger::Manual && self.requires_risk_confirmation(idx) {
            self.pending_start_confirmation = Some(PendingStartConfirmation {
                job_id: self.config.jobs[idx].id,
                trigger,
                retry_attempt,
            });
            ctx.request_repaint();
            return;
        }
        self.start_sync_entry(idx, trigger, retry_attempt, ctx);
    }

    /// 检查预览结果是否就绪
    fn drain_preview(&mut self) {
        let result = match &self.preview_rx {
            Some(rx) => rx.try_recv().ok(),
            None => return,
        };
        if let Some(r) = result {
            self.preview_rx = None;
            self.preview_state = match r {
                Ok(entries) => PreviewState::Ready(entries),
                Err(e) => PreviewState::Error(e),
            };
        }
    }

    /// 发送停止信号（同时清空任务队列）
    pub fn stop_sync(&mut self) {
        if let Some(s) = &self.stop_signal {
            s.store(true, Ordering::Relaxed);
        }
        if let Some(session) = &mut self.session {
            session.status = SessionStatus::Stopped;
        }
        self.sync_running = false;
        self.pending_delete_fallbacks.clear();
        self.pending_mass_delete_confirmation = None;
        self.job_queue.clear();
        self.pending_queue_start = false;
        self.pending_start_confirmation = None;
    }

    /// 从 channel 中取出待处理事件，更新 session 状态。
    /// 每帧最多处理 MAX_EVENTS_PER_FRAME 条，防止突发大量事件阻塞 UI 帧。
    /// 若还有剩余事件，由 update() 中的 request_repaint() 驱动下一帧继续处理。
    fn drain_events(&mut self) {
        const MAX_EVENTS_PER_FRAME: usize = 2000;
        // Clone the receiver (cheap Arc clone) to release the shared borrow on self,
        // allowing the mutable borrow in handle_event() below.
        let rx = match self.event_rx.clone() {
            Some(rx) => rx,
            None => return,
        };
        for _ in 0..MAX_EVENTS_PER_FRAME {
            match rx.try_recv() {
                Ok(event) => self.handle_event(event),
                Err(_) => break,
            }
        }
    }

    fn handle_event(&mut self, event: SyncEvent) {
        let Some(mut session) = self.session.take() else { return };

        match event {
            SyncEvent::Started { total_files, total_bytes } => {
                Self::handle_started(&mut session, total_files, total_bytes);
            }
            SyncEvent::FileStarted { worker_id, path, size, is_new } => {
                Self::handle_file_started(&mut session, worker_id, path, size, is_new);
            }
            SyncEvent::FileProgress { worker_id, bytes_done } => {
                Self::handle_file_progress(&mut session, worker_id, bytes_done);
            }
            SyncEvent::DeleteStarted { worker_id, path, is_dir } => {
                Self::handle_delete_started(&mut session, worker_id, path, is_dir);
            }
            SyncEvent::DeleteFallbackRequired {
                path,
                is_dir,
                message,
                response,
            } => {
                self.pending_delete_fallbacks.push_back(PendingDeleteFallback {
                    path,
                    is_dir,
                    message,
                    response,
                });
            }
            SyncEvent::MassDeleteConfirmationRequired { count, response } => {
                self.pending_mass_delete_confirmation =
                    Some(PendingMassDeleteConfirmation { count, response });
            }
            SyncEvent::FileCompleted { worker_id, path, size, delta, saved_bytes, .. } => {
                Self::handle_file_completed(
                    &mut session,
                    worker_id,
                    path,
                    size,
                    delta,
                    saved_bytes,
                );
            }
            SyncEvent::FileSkipped { .. } => Self::handle_file_skipped(&mut session),
            SyncEvent::FileDeleted { worker_id, path } => {
                Self::handle_file_deleted(&mut session, worker_id, path)
            }
            SyncEvent::WorkerFinished { worker_id } => {
                Self::handle_worker_finished(&mut session, worker_id);
            }
            SyncEvent::FileOrphan { path } => Self::handle_file_orphan(&mut session, path),
            SyncEvent::FileError { path, message, scope } => {
                Self::handle_file_error(&mut session, path, message, scope);
            }
            SyncEvent::Completed { stats, usn_checkpoints, was_stopped } => {
                flow::handle_sync_completed(self, &mut session, stats, usn_checkpoints, was_stopped);
            }
            SyncEvent::StartFailed { message } => {
                flow::handle_start_failed(self, &mut session, message);
            }
            SyncEvent::DiskFull => flow::handle_disk_full(self, &mut session),
            SyncEvent::Paused => session.status = SessionStatus::Paused,
            SyncEvent::Resumed => session.status = SessionStatus::Running,
            SyncEvent::SpeedUpdate { bps: _ } => {}
        }

        self.session = Some(session);
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
        session.copied_log.push(crate::model::session::CopiedFileEntry { path, size, delta });
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
            timestamp: Utc::now(),
            path,
            scope,
            kind: ErrorKind::IoError,
            message,
        });
    }
}


fn has_partial_enabled_folder_pair(
    folder_pairs: &[crate::model::job::FolderPair],
) -> bool {
    folder_pairs.iter().any(|pair| {
        pair.enabled
            && (pair.source.as_os_str().is_empty()
                != pair.destination.as_os_str().is_empty())
    })
}

fn has_valid_enabled_folder_pair(
    folder_pairs: &[crate::model::job::FolderPair],
) -> bool {
    folder_pairs.iter().any(|pair| {
        pair.enabled
            && !pair.source.as_os_str().is_empty()
            && !pair.destination.as_os_str().is_empty()
    })
}

fn completed_session_status(was_stopped: bool) -> SessionStatus {
    if was_stopped {
        SessionStatus::Stopped
    } else {
        SessionStatus::Completed
    }
}

fn should_record_sync_completion(was_stopped: bool) -> bool {
    !was_stopped
}

pub(crate) fn effective_copied_bytes(session: &SyncSession) -> u64 {
    session.stats.copied_bytes
        + session
            .active_workers
            .iter()
            .map(|worker| match worker {
                WorkerState::Copying { done, .. } => *done,
                WorkerState::Deleting { .. } | WorkerState::Idle => 0,
            })
            .sum::<u64>()
}

fn is_schedule_due(
    last_sync_time: Option<DateTime<Utc>>,
    interval_minutes: u32,
    now: DateTime<Utc>,
) -> bool {
    match last_sync_time {
        Some(last) => {
            now >= last + chrono::Duration::minutes(interval_minutes as i64)
        }
        None => true,
    }
}

fn build_completion_notification(
    finished_job_name: &str,
    copied: u64,
    skipped: u64,
    errors: u64,
    deleted: u64,
    zh: bool,
) -> AppNotification {
    let mut body_parts = if zh {
        vec![format!("复制 {} 个", copied), format!("跳过 {} 个", skipped)]
    } else {
        vec![format!("Copied {}", copied), format!("Skipped {}", skipped)]
    };

    if errors > 0 {
        body_parts.push(if zh {
            format!("错误 {} 个", errors)
        } else {
            format!("Errors {}", errors)
        });
    }
    if deleted > 0 {
        body_parts.push(if zh {
            format!("删除 {} 个", deleted)
        } else {
            format!("Deleted {}", deleted)
        });
    }

    AppNotification {
        title: if zh {
            format!("「{}」同步完成", finished_job_name)
        } else {
            format!("\"{}\" sync complete", finished_job_name)
        },
        body: body_parts.join("  "),
        created_at: std::time::Instant::now(),
        kind: if errors > 0 {
            NotificationKind::Warning
        } else {
            NotificationKind::Success
        },
    }
}



#[cfg(test)]
mod tests {
    use super::{
        build_completion_notification, completed_session_status, has_partial_enabled_folder_pair,
        has_valid_enabled_folder_pair, is_schedule_due, schedule, should_record_sync_completion,
        NotificationKind,
    };
    use crate::model::config::AppConfig;
    use crate::model::job::{FolderPair, SyncJob, SyncSchedule};
    use crate::model::runtime::{JobStateRecord, ScheduleRuntimeState};
    use crate::model::session::SessionStatus;
    use chrono::{Duration, Utc};

    #[test]
    fn completed_status_reflects_stop_flag() {
        assert!(matches!(completed_session_status(true), SessionStatus::Stopped));
        assert!(matches!(completed_session_status(false), SessionStatus::Completed));
    }

    #[test]
    fn record_sync_completion_skips_user_stop() {
        assert!(!should_record_sync_completion(true));
        assert!(should_record_sync_completion(false));
    }

    #[test]
    fn schedule_is_due_only_after_interval() {
        let now = Utc::now();
        let recent = now - Duration::minutes(10);
        let old = now - Duration::minutes(31);

        assert!(!is_schedule_due(Some(recent), 30, now));
        assert!(is_schedule_due(Some(old), 30, now));
    }

    #[test]
    fn completion_notification_uses_success_style_without_errors() {
        let notification = build_completion_notification("Job A", 3, 2, 0, 0, false);
        assert_eq!(notification.title, "\"Job A\" sync complete");
        assert_eq!(notification.body, "Copied 3  Skipped 2");
        assert_eq!(notification.kind, NotificationKind::Success);
    }

    #[test]
    fn completion_notification_uses_warning_style_with_errors_and_deletes() {
        let notification = build_completion_notification("任务A", 5, 1, 2, 4, true);
        assert_eq!(notification.title, "「任务A」同步完成");
        assert_eq!(notification.body, "复制 5 个  跳过 1 个  错误 2 个  删除 4 个");
        assert_eq!(notification.kind, NotificationKind::Warning);
    }

    #[test]
    fn has_enabled_schedule_ignores_disabled_and_zero_interval_jobs() {
        let mut config = AppConfig::default();

        let mut disabled = SyncJob::new("disabled".into(), 1);
        disabled.schedule = SyncSchedule { enabled: false, interval_minutes: 30, ..SyncSchedule::default() };

        let mut zero_interval = SyncJob::new("zero".into(), 1);
        zero_interval.schedule = SyncSchedule { enabled: true, interval_minutes: 0, ..SyncSchedule::default() };

        let mut active = SyncJob::new("active".into(), 1);
        active.schedule = SyncSchedule { enabled: true, interval_minutes: 15, ..SyncSchedule::default() };

        config.jobs = vec![disabled, zero_interval];
        assert!(!schedule::has_enabled_schedule(&config));

        config.jobs.push(active);
        assert!(schedule::has_enabled_schedule(&config));
    }

    #[test]
    fn paused_or_unacknowledged_schedule_is_not_counted() {
        let mut config = AppConfig::default();

        let mut paused = SyncJob::new("paused".into(), 1);
        paused.schedule = SyncSchedule { enabled: true, interval_minutes: 15, ..SyncSchedule::default() };

        let mut unack_mirror = SyncJob::new("mirror".into(), 1);
        unack_mirror.sync_mode = crate::model::job::SyncMode::Mirror;
        unack_mirror.schedule = SyncSchedule { enabled: true, interval_minutes: 15, risk_acknowledged: false, ..SyncSchedule::default() };

        config.jobs = vec![paused.clone(), unack_mirror.clone()];
        config.job_states = vec![
            JobStateRecord {
                job_id: paused.id,
                schedule_runtime: ScheduleRuntimeState { paused: true, ..ScheduleRuntimeState::default() },
                ..JobStateRecord::default()
            },
            JobStateRecord { job_id: unack_mirror.id, ..JobStateRecord::default() },
        ];
        assert!(!schedule::has_enabled_schedule(&config));
    }

    #[test]
    fn collect_due_scheduled_jobs_orders_oldest_last_run_first() {
        let now = Utc::now();
        let mut config = AppConfig::default();

        let mut recent = SyncJob::new("recent".into(), 1);
        recent.schedule = SyncSchedule { enabled: true, interval_minutes: 15, ..SyncSchedule::default() };
        let recent_time = now - Duration::minutes(20);

        let mut oldest = SyncJob::new("oldest".into(), 1);
        oldest.schedule = SyncSchedule { enabled: true, interval_minutes: 15, ..SyncSchedule::default() };
        let oldest_time = now - Duration::minutes(60);

        let mut not_due = SyncJob::new("not_due".into(), 1);
        not_due.schedule = SyncSchedule { enabled: true, interval_minutes: 30, ..SyncSchedule::default() };
        let not_due_time = now - Duration::minutes(10);

        config.jobs = vec![recent.clone(), oldest.clone(), not_due.clone()];
        config.job_states = vec![
            JobStateRecord { job_id: recent.id, last_sync_time: Some(recent_time), ..JobStateRecord::default() },
            JobStateRecord { job_id: oldest.id, last_sync_time: Some(oldest_time), ..JobStateRecord::default() },
            JobStateRecord { job_id: not_due.id, last_sync_time: Some(not_due_time), ..JobStateRecord::default() },
        ];
        assert_eq!(schedule::collect_due_scheduled_jobs_at(&config, now), vec![1, 0]);
    }

    #[test]
    fn collect_due_scheduled_jobs_skips_paused_and_unacknowledged_mirror() {
        let now = Utc::now();
        let mut config = AppConfig::default();

        let mut paused = SyncJob::new("paused".into(), 1);
        paused.schedule = SyncSchedule { enabled: true, interval_minutes: 15, ..SyncSchedule::default() };
        let paused_time = now - Duration::minutes(60);

        let mut mirror = SyncJob::new("mirror".into(), 1);
        mirror.sync_mode = crate::model::job::SyncMode::Mirror;
        mirror.schedule = SyncSchedule { enabled: true, interval_minutes: 15, risk_acknowledged: false, ..SyncSchedule::default() };
        let mirror_time = now - Duration::minutes(60);

        let mut valid = SyncJob::new("valid".into(), 1);
        valid.schedule = SyncSchedule { enabled: true, interval_minutes: 15, ..SyncSchedule::default() };
        let valid_time = now - Duration::minutes(60);

        config.jobs = vec![paused.clone(), mirror.clone(), valid.clone()];
        config.job_states = vec![
            JobStateRecord {
                job_id: paused.id,
                last_sync_time: Some(paused_time),
                schedule_runtime: ScheduleRuntimeState { paused: true, ..ScheduleRuntimeState::default() },
                ..JobStateRecord::default()
            },
            JobStateRecord { job_id: mirror.id, last_sync_time: Some(mirror_time), ..JobStateRecord::default() },
            JobStateRecord { job_id: valid.id, last_sync_time: Some(valid_time), ..JobStateRecord::default() },
        ];
        assert_eq!(schedule::collect_due_scheduled_jobs_at(&config, now), vec![2]);
    }

    #[test]
    fn folder_pair_helpers_distinguish_partial_and_valid_pairs() {
        let mut partial = FolderPair::new();
        partial.source = "C:\\src".into();

        let mut valid = FolderPair::new();
        valid.source = "C:\\src".into();
        valid.destination = "D:\\dst".into();

        let disabled_empty = FolderPair { enabled: false, ..FolderPair::new() };

        assert!(has_partial_enabled_folder_pair(&[partial.clone()]));
        assert!(!has_valid_enabled_folder_pair(&[partial]));
        assert!(has_valid_enabled_folder_pair(&[valid.clone()]));
        assert!(!has_partial_enabled_folder_pair(&[valid]));
        assert!(!has_partial_enabled_folder_pair(&[disabled_empty.clone()]));
        assert!(!has_valid_enabled_folder_pair(&[disabled_empty]));
    }
}

#[cfg(test)]
mod regression_tests {
    use super::{flow, FileSyncApp, QueueEntry};
    use crate::model::config::AppConfig;
    use crate::model::job::{RunResultStatus, RunTrigger, SyncJob};
    use crate::model::preview::PreviewState;
    use crate::model::runtime::JobTransientState;
    use chrono::{Duration, Utc};
    use std::collections::{HashMap, VecDeque};

    fn new_test_app(config: AppConfig) -> FileSyncApp {
        let mut job_transient = HashMap::new();
        for job in &config.jobs {
            job_transient.insert(job.id, JobTransientState::default());
        }

        FileSyncApp {
            config,
            selected_job: None,
            settings_dirty: false,
            session: None,
            sync_running: false,
            pending_delete: None,
            new_exclusion_input: String::new(),
            exclusion_error: None,
            error_message: None,
            settings_open: false,
            about_open: false,
            preview_state: PreviewState::Idle,
            preview_job_is_mirror: false,
            event_rx: None,
            preview_rx: None,
            job_queue: VecDeque::new(),
            job_transient,
            pending_queue_start: false,
            stop_signal: None,
            notification: None,
            tray: None,
            close_dialog_open: false,
            close_dialog_remember: false,
            unsaved_dialog_open: false,
            progress_panel_height: None,
            pending_delete_fallbacks: VecDeque::new(),
            pending_mass_delete_confirmation: None,
            pending_start_confirmation: None,
            history_open: false,
        }
    }

    #[test]
    fn enqueue_job_orders_by_ready_time_and_deduplicates() {
        let config = AppConfig::default();
        let mut app = new_test_app(config);
        let job_a = uuid::Uuid::new_v4();
        let job_b = uuid::Uuid::new_v4();
        let now = Utc::now();

        app.enqueue_job(QueueEntry {
            job_id: job_a,
            trigger: RunTrigger::Manual,
            retry_attempt: 0,
            ready_at: now + Duration::minutes(10),
        });
        app.enqueue_job(QueueEntry {
            job_id: job_b,
            trigger: RunTrigger::Scheduled,
            retry_attempt: 0,
            ready_at: now + Duration::minutes(5),
        });
        app.enqueue_job(QueueEntry {
            job_id: job_a,
            trigger: RunTrigger::Retry,
            retry_attempt: 1,
            ready_at: now + Duration::minutes(1),
        });

        assert_eq!(app.job_queue.len(), 2);
        assert_eq!(app.job_queue[0].job_id, job_b);
        assert_eq!(app.job_queue[1].job_id, job_a);
        assert!(app.pending_queue_start);
    }

    #[test]
    fn apply_run_outcome_pauses_after_consecutive_scheduled_failures() {
        let mut config = AppConfig::default();
        let mut job = SyncJob::new("job".into(), 1);
        job.schedule.pause_after_failures = 2;
        let job_id = job.id;
        config.jobs.push(job);
        let mut app = new_test_app(config);

        flow::apply_run_outcome(
            &mut app,
            0,
            RunTrigger::Scheduled,
            RunResultStatus::Warning,
            "first failure",
        );
        assert_eq!(
            app.job_state(job_id).unwrap().schedule_runtime.consecutive_failures,
            1
        );
        assert!(!app.job_state(job_id).unwrap().schedule_runtime.paused);

        flow::apply_run_outcome(
            &mut app,
            0,
            RunTrigger::Scheduled,
            RunResultStatus::Failed,
            "second failure",
        );
        let runtime = &app.job_state(job_id).unwrap().schedule_runtime;
        assert_eq!(runtime.consecutive_failures, 2);
        assert!(runtime.paused);
        assert!(runtime.pause_reason.contains("2"));
    }

    #[test]
    fn apply_run_outcome_success_resets_pause_state() {
        let mut config = AppConfig::default();
        let mut job = SyncJob::new("job".into(), 1);
        job.schedule.pause_after_failures = 1;
        let job_id = job.id;
        config.jobs.push(job);
        let mut app = new_test_app(config);

        flow::apply_run_outcome(
            &mut app,
            0,
            RunTrigger::Scheduled,
            RunResultStatus::Failed,
            "failure",
        );
        assert!(app.job_state(job_id).unwrap().schedule_runtime.paused);

        flow::apply_run_outcome(
            &mut app,
            0,
            RunTrigger::Manual,
            RunResultStatus::Completed,
            "",
        );
        let runtime = &app.job_state(job_id).unwrap().schedule_runtime;
        assert_eq!(runtime.consecutive_failures, 0);
        assert!(!runtime.paused);
        assert!(runtime.pause_reason.is_empty());
    }
}
