use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::config::storage;
use crate::engine::events::SyncEvent;
use crate::i18n::{is_zh, t};
use crate::model::config::{AppConfig, CompareMethod, Theme};
use crate::model::preview::{PreviewEntry, PreviewState};
use crate::model::session::{ErrorKind, SessionStatus, SyncError, SyncSession, WorkerState};
use crate::ui::{job_editor, job_list, preview, progress};

use crate::log::LogLevel;

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
    pub job_queue: std::collections::VecDeque<usize>,
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
            pending_queue_start: false,
            stop_signal: None,
            notification: None,
            tray,
            close_dialog_open: false,
            close_dialog_remember: false,
            unsaved_dialog_open: false,
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
        self.settings_dirty || self.config.jobs.iter().any(|j| j.dirty)
    }

    /// 当前选中 job 是否有未保存的修改
    pub fn current_job_dirty(&self) -> bool {
        self.selected_job
            .map(|idx| self.config.jobs.get(idx).map(|j| j.dirty).unwrap_or(false))
            .unwrap_or(false)
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
                for job in &mut self.config.jobs {
                    job.dirty = false;
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
        if let Some(idx) = self.selected_job {
            if let Some(err) = self.validate_folder_pairs_for_start(idx) {
                self.error_message = Some(err);
                return false;
            }
        }
        self.start_sync(ctx);
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
    pub fn start_sync(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.selected_job else { return };
        if idx >= self.config.jobs.len() {
            return;
        }

        let job = self.config.jobs[idx].clone();
        let concurrency = job.concurrency.max(1);

        let (tx, rx) = flume::bounded(4096);
        let stop = Arc::new(AtomicBool::new(false));

        self.event_rx = Some(rx);
        self.stop_signal = Some(stop.clone());
        self.sync_running = true;
        self.session = Some(SyncSession::new(job.id, concurrency));

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
            rt.block_on(crate::engine::executor::run_sync(
                job,
                tx,
                ctx_clone,
                stop,
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
        self.job_queue.clear();
        self.pending_queue_start = false;
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
            SyncEvent::FileDeleted { path } => Self::handle_file_deleted(&mut session, path),
            SyncEvent::FileOrphan { path } => Self::handle_file_orphan(&mut session, path),
            SyncEvent::FileError { path, message } => {
                Self::handle_file_error(&mut session, path, message);
            }
            SyncEvent::Completed { stats, usn_checkpoints, was_stopped } => {
                self.handle_sync_completed(&mut session, stats, usn_checkpoints, was_stopped);
            }
            SyncEvent::StartFailed { message } => {
                self.handle_start_failed(&mut session, message);
            }
            SyncEvent::DiskFull => self.handle_disk_full(&mut session),
            SyncEvent::Paused => session.status = SessionStatus::Paused,
            SyncEvent::Resumed => session.status = SessionStatus::Running,
            SyncEvent::SpeedUpdate { bps } => session.stats.speed_bps = bps,
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
    }

    fn handle_file_skipped(session: &mut SyncSession) {
        session.stats.skipped_files += 1;
        session.stats.processed_files += 1;
    }

    fn handle_file_deleted(session: &mut SyncSession, path: std::path::PathBuf) {
        session.stats.deleted_files += 1;
        session.deleted_paths.push(path);
    }

    fn handle_file_orphan(session: &mut SyncSession, path: std::path::PathBuf) {
        session.orphan_log.push(path);
    }

    fn handle_file_error(
        session: &mut SyncSession,
        path: std::path::PathBuf,
        message: String,
    ) {
        session.stats.error_count += 1;
        session.stats.processed_files += 1;
        session.errors.push(SyncError {
            timestamp: Utc::now(),
            path,
            kind: ErrorKind::IoError,
            message,
        });
    }

    fn handle_sync_completed(
        &mut self,
        session: &mut SyncSession,
        stats: crate::model::session::SyncStats,
        usn_checkpoints: std::collections::HashMap<String, (u64, i64)>,
        was_stopped: bool,
    ) {
        let elapsed_secs = (Utc::now() - session.started_at).num_seconds().max(0) as u64;
        let summary = crate::model::job::RunSummary {
            copied: stats.copied_files,
            skipped: stats.skipped_files,
            errors: stats.error_count,
            deleted: stats.deleted_files,
            bytes: stats.copied_bytes,
            elapsed_secs,
        };
        let n_copied = summary.copied;
        let n_skipped = summary.skipped;
        let n_errors = summary.errors;
        let n_deleted = summary.deleted;

        let finished_job_idx = self.selected_job;
        let finished_job_name = finished_job_idx
            .and_then(|i| self.config.jobs.get(i))
            .map(|j| j.name.clone())
            .unwrap_or_default();

        session.stats = stats;
        session.status = completed_session_status(was_stopped);
        for w in &mut session.active_workers {
            *w = WorkerState::Idle;
        }

        let log_data = crate::log::SyncLogData {
            job_name: &finished_job_name,
            started_at: session.started_at,
            finished_at: Utc::now(),
            stats: &session.stats,
            copied_log: &session.copied_log,
            deleted_log: &session.deleted_paths,
            orphan_log: &session.orphan_log,
            errors: &session.errors,
        };
        if let Err(e) = crate::log::write_sync_log(&log_data) {
            crate::log::app_log(
                &format!("write_sync_log failed: {}", e),
                crate::log::LogLevel::Error,
            );
        }

        self.sync_running = false;
        self.stop_signal = None;

        if should_record_sync_completion(was_stopped) {
            play_completion_sound();
        }

        if let Some(idx) = finished_job_idx {
            if let Some(job) = self.config.jobs.get_mut(idx) {
                if should_record_sync_completion(was_stopped) {
                    job.last_sync_time = Some(Utc::now());
                    job.last_run_summary = Some(summary);
                    // 刷新本进程内的 USN 检查点。
                    // `last_sync_checkpoints` 带有 `#[serde(skip)]`，不会写入磁盘；
                    // 应用重启后会从空检查点重新开始。
                    if !usn_checkpoints.is_empty() {
                        for (vol, (journal_id, next_usn)) in usn_checkpoints {
                            job.last_sync_checkpoints.insert(
                                vol,
                                crate::model::job::UsnCheckpoint { journal_id, next_usn },
                            );
                        }
                    }
                    // 运行统计自动保存，不标记 dirty（用户未修改配置）。
                    // 这里持久化的是 last_sync_time / last_run_summary，不包含 USN 检查点。
                    if let Err(e) = crate::config::storage::save(&self.config) {
                        crate::log::app_log(
                            &format!("auto-save after sync failed (run summary may be lost): {}", e),
                            LogLevel::Error,
                        );
                    }
                }
            }
        }

        if should_record_sync_completion(was_stopped) {
            if let Some(next_idx) = self.job_queue.pop_front() {
                self.selected_job = Some(next_idx);
                self.pending_queue_start = true;
            }

            self.notification = Some(build_completion_notification(
                &finished_job_name,
                n_copied,
                n_skipped,
                n_errors,
                n_deleted,
                is_zh(),
            ));
        }
    }

    fn handle_start_failed(&mut self, session: &mut SyncSession, message: String) {
        session.status = SessionStatus::Failed;
        for w in &mut session.active_workers {
            *w = WorkerState::Idle;
        }
        self.sync_running = false;
        self.stop_signal = None;
        self.error_message = Some(if is_zh() {
            format!("启动同步失败: {}", message)
        } else {
            format!("Failed to start sync: {}", message)
        });
    }

    fn handle_disk_full(&mut self, session: &mut SyncSession) {
        session.status = SessionStatus::Failed;
        self.sync_running = false;
        self.stop_signal = None;
        self.error_message = Some(
            t("磁盘空间不足，同步已停止！", "Disk full — sync stopped!").into(),
        );
    }

    /// 检查是否有定时任务到期，返回第一个到期的任务索引
    fn check_scheduled_sync(&self) -> Option<usize> {
        if self.sync_running {
            return None;
        }
        let now = Utc::now();
        for (i, job) in self.config.jobs.iter().enumerate() {
            if !job.schedule.enabled || job.schedule.interval_minutes == 0 {
                continue;
            }
            if is_schedule_due(job.last_sync_time, job.schedule.interval_minutes, now) {
                return Some(i);
            }
        }
        None
    }

    /// 应用当前主题设置
    fn apply_theme(&self, ctx: &egui::Context) {
        match self.config.settings.theme {
            Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
            Theme::Light => ctx.set_visuals(egui::Visuals::light()),
            Theme::System => {
                // 跟随系统：eframe 默认已是系统主题，此处不覆盖
            }
        }
    }

    fn handle_close_requests(&mut self, ctx: &egui::Context) {
        if crate::tray::close_button_clicked() {
            crate::tray::reset_close_button();
            self.handle_close_button_click();
        }

        if self.force_quit_requested() {
            self.quit_app();
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            self.quit_app();
        }
    }

    fn handle_close_button_click(&mut self) {
        use crate::model::config::CloseAction;

        if self.close_dialog_open {
            return;
        }

        match &self.config.settings.close_action {
            CloseAction::MinimizeToTray if self.tray.is_some() => {
                crate::tray::hide_app_window();
            }
            CloseAction::Ask if self.tray.is_some() => {
                self.close_dialog_open = true;
            }
            _ => self.quit_app(),
        }
    }

    fn force_quit_requested(&self) -> bool {
        self.tray
            .as_ref()
            .map_or(false, |t| t.force_quit.load(std::sync::atomic::Ordering::Acquire))
    }

    fn show_close_dialog(&mut self, ctx: &egui::Context) {
        let mut remember = self.close_dialog_remember;
        let mut action: Option<CloseDialogAction> = None;

        egui::Window::new(t("关闭 FileSync", "Close FileSync"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                if self.sync_running {
                    ui.label(
                        egui::RichText::new(t(
                            "? 同步正在进行中，退出将中断当前同步。",
                            "? Sync is in progress. Quitting will interrupt it.",
                        ))
                        .color(egui::Color32::from_rgb(255, 180, 50)),
                    );
                    ui.add_space(8.0);
                }
                ui.label(t("请选择关闭行为：", "Choose what to do:"));
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button(t("最小化到托盘", "Minimize to Tray")).clicked() {
                        action = Some(CloseDialogAction::Minimize);
                    }
                    ui.add_space(8.0);
                    if ui.button(t("退出程序", "Quit")).clicked() {
                        action = Some(CloseDialogAction::Quit);
                    }
                    ui.add_space(8.0);
                    if ui.button(t("取消", "Cancel")).clicked() {
                        action = Some(CloseDialogAction::Cancel);
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                ui.checkbox(
                    &mut remember,
                    t(
                        "下次不再询问（可在设置中修改）",
                        "Remember my choice (can be changed in Settings)",
                    ),
                );
            });

        self.close_dialog_remember = remember;

        match action {
            Some(CloseDialogAction::Minimize) => {
                self.close_dialog_open = false;
                if self.tray.is_some() {
                    if self.close_dialog_remember {
                        self.config.settings.close_action =
                            crate::model::config::CloseAction::MinimizeToTray;
                        self.settings_dirty = true;
                        self.save();
                    }
                    crate::tray::hide_app_window();
                } else {
                    crate::log::app_log(
                        "close dialog: minimize requested but no tray, quitting instead",
                        LogLevel::Info,
                    );
                    self.quit_app();
                }
            }
            Some(CloseDialogAction::Quit) => {
                self.close_dialog_open = false;
                if self.close_dialog_remember {
                    self.config.settings.close_action = crate::model::config::CloseAction::Quit;
                    self.settings_dirty = true;
                }
                self.quit_app();
            }
            Some(CloseDialogAction::Cancel) => {
                self.close_dialog_open = false;
                self.close_dialog_remember = false;
            }
            None => {}
        }
    }

    fn start_pending_queued_job(&mut self, ctx: &egui::Context) {
        if self.pending_queue_start && !self.sync_running {
            self.pending_queue_start = false;
            self.start_sync(ctx);
        }
    }

    fn trigger_scheduled_sync_if_due(&mut self, ctx: &egui::Context) {
        if let Some(idx) = self.check_scheduled_sync() {
            self.selected_job = Some(idx);
            self.save();
            self.start_sync(ctx);
        }
    }

    fn request_schedule_wake_if_needed(&self, ctx: &egui::Context) {
        if has_enabled_schedule(&self.config) {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let want_save = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S));
        let want_sync = ctx.input(|i| i.key_pressed(egui::Key::F5));

        if want_save {
            self.save_selected_job_with_validation();
        }
        if want_sync && !self.sync_running {
            self.start_selected_sync_with_validation(ctx);
        }
    }

    fn show_error_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.error_message.clone() else {
            return;
        };

        egui::Window::new(t("错误", "Error"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&msg);
                ui.add_space(8.0);
                if ui.button(t("确定", "OK")).clicked() {
                    self.error_message = None;
                }
            });
    }

    fn show_unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        if !self.unsaved_dialog_open {
            return;
        }

        let mut keep_open = true;
        egui::Window::new(t("未保存的修改", "Unsaved Changes"))
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t(
                    "当前有未保存的修改，退出前是否保存？",
                    "You have unsaved changes. Save before quitting?",
                ));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(t("保存", "Save")).clicked() {
                        self.save();
                        self.unsaved_dialog_open = false;
                        self.quit_app_now();
                    }
                    if ui.button(t("不保存", "Don't Save")).clicked() {
                        self.unsaved_dialog_open = false;
                        self.quit_app_now();
                    }
                    if ui.button(t("取消", "Cancel")).clicked() {
                        self.unsaved_dialog_open = false;
                    }
                });
            });

        if !keep_open {
            self.unsaved_dialog_open = false;
        }
    }
}

fn has_enabled_schedule(config: &AppConfig) -> bool {
    config
        .jobs
        .iter()
        .any(|j| j.schedule.enabled && j.schedule.interval_minutes > 0)
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

impl eframe::App for FileSyncApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::log::app_log("on_exit() called", LogLevel::Info);
        self.tray = None;
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        static FIRST_UPDATE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
        if FIRST_UPDATE.swap(false, std::sync::atomic::Ordering::Relaxed) {
            crate::log::app_log("update() first frame", LogLevel::Info);
        }

        // 首帧在主线程安装 wndproc 钩子（SetWindowLongPtr 必须在窗口所属线程调用）
        crate::tray::install_close_hook_once();

        // ── 关闭按钮处理 ──────────────────────────────────────────
        //
        // 路径 A：close_button_clicked()
        //   wndproc 钩子拦截 SC_CLOSE 后置此标志，同时吃掉 SC_CLOSE 和后续 WM_CLOSE，
        //   使 eframe 完全不感知本次关闭，由此处按 CloseAction 分发。
        //
        // 路径 B：force_quit（托盘"退出"菜单）
        //   无视 CloseAction，直接退出。
        //
        // 路径 C：close_requested()
        //   钩子未安装时的保底路径（eframe 已收到 WM_CLOSE 并开始关闭流程），
        //   此时不再尝试显示对话框，直接退出以配合 eframe 的关闭。

        self.handle_close_requests(ctx);

        // ── 关闭确认对话框 ────────────────────────────────────────
        if self.close_dialog_open {
            self.show_close_dialog(ctx);
        }

        self.apply_theme(ctx);
        self.drain_events();
        self.drain_preview();

        self.start_pending_queued_job(ctx);
        self.trigger_scheduled_sync_if_due(ctx);

        if self.sync_running {
            ctx.request_repaint();
        } else {
            // 有启用的定时任务时，每 30 秒唤醒一次以检查到期
            self.request_schedule_wake_if_needed(ctx);
        }

        // ── 顶部状态栏 ────────────────────────────────────────────
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("FileSync");
                ui.separator();
                if self.is_dirty() {
                    ui.label(
                        egui::RichText::new(t("● 有未保存的修改", "● Unsaved changes"))
                            .color(egui::Color32::from_rgb(255, 180, 80))
                            .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t("ℹ 关于", "ℹ About")).clicked() {
                        self.about_open = !self.about_open;
                    }
                    if ui.small_button(t("⚙ 设置", "⚙ Settings")).clicked() {
                        self.settings_open = !self.settings_open;
                    }
                    ui.label(
                        egui::RichText::new(t("Ctrl+S 保存  F5 同步", "Ctrl+S Save  F5 Sync"))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            });
        });

        // ── 底部进度面板 ──────────────────────────────────────────
        let progress_default_h = (ctx.screen_rect().height() * 0.30).clamp(200.0, 300.0);
        egui::TopBottomPanel::bottom("progress_panel")
            .resizable(true)
            .min_height(120.0)
            .default_height(progress_default_h)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("progress_scroll")
                    .show(ui, |ui| {
                        progress::show(ui, self);
                    });
            });

        // ── 左侧任务列表 ──────────────────────────────────────────
        egui::SidePanel::left("job_list_panel")
            .resizable(false)
            .exact_width(210.0)
            .show(ctx, |ui| {
                job_list::show(ui, self);
            });

        // ── 主内容区 ──────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            job_editor::show(ui, self);
        });

        // ── 设置窗口 ──────────────────────────────────────────────
        if self.settings_open {
            let mut open = self.settings_open;
            egui::Window::new(t("设置", "Settings"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .min_width(280.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);

                    // 主题
                    ui.strong(t("界面主题", "Theme"));
                    ui.add_space(4.0);
                    let theme_options: &[(&str, Theme)] = if is_zh() {
                        &[
                            ("跟随系统", Theme::System),
                            ("浅色", Theme::Light),
                            ("深色", Theme::Dark),
                        ]
                    } else {
                        &[
                            ("Follow System", Theme::System),
                            ("Light", Theme::Light),
                            ("Dark", Theme::Dark),
                        ]
                    };
                    for (label, variant) in theme_options {
                        if ui
                            .radio(self.config.settings.theme == *variant, *label)
                            .clicked()
                        {
                            self.config.settings.theme = variant.clone();
                            self.settings_dirty = true;
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // 默认并发数
                    ui.strong(t("新建任务默认并发数", "Default Concurrency for New Jobs"));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let mut c = self.config.settings.default_concurrency;
                        if ui
                            .add(
                                egui::Slider::new(&mut c, 1usize..=16)
                                    .text(t("线程", "threads")),
                            )
                            .changed()
                        {
                            self.config.settings.default_concurrency = c;
                            self.settings_dirty = true;
                        }
                    });

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // 关闭行为（仅托盘可用时有意义）
                    if self.tray.is_some() {
                        ui.strong(t("点击关闭按钮（X）时", "When clicking Close (X)"));
                        ui.add_space(4.0);
                        let close_options: &[(&str, crate::model::config::CloseAction)] =
                            if is_zh() {
                                &[
                                    ("每次询问", crate::model::config::CloseAction::Ask),
                                    (
                                        "最小化到托盘",
                                        crate::model::config::CloseAction::MinimizeToTray,
                                    ),
                                    ("退出程序", crate::model::config::CloseAction::Quit),
                                ]
                            } else {
                                &[
                                    ("Ask every time", crate::model::config::CloseAction::Ask),
                                    (
                                        "Minimize to tray",
                                        crate::model::config::CloseAction::MinimizeToTray,
                                    ),
                                    ("Quit", crate::model::config::CloseAction::Quit),
                                ]
                            };
                        for (label, variant) in close_options {
                            if ui
                                .radio(
                                    self.config.settings.close_action == *variant,
                                    *label,
                                )
                                .clicked()
                            {
                                self.config.settings.close_action = variant.clone();
                                self.settings_dirty = true;
                            }
                        }
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }

                    // 配置备份/还原
                    ui.strong(t("配置备份", "Config Backup"));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button(t("📤 导出配置", "📤 Export Config")).clicked() {
                            export_config(&self.config);
                        }
                        ui.add_space(8.0);
                        if ui.button(t("📥 导入配置", "📥 Import Config")).clicked() {
                            if let Some(imported) = import_config() {
                                self.config = imported;
                                self.settings_dirty = false;
                                self.selected_job = None;
                            }
                        }
                    });
                    ui.label(
                        egui::RichText::new(t(
                            "导入将覆盖当前所有任务和设置",
                            "Import will overwrite all current jobs and settings",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        if ui.button(t("💾 保存", "💾 Save")).clicked() {
                            self.save();
                        }
                    });
                });
            self.settings_open = open;
        }

        // ── 预览窗口 ──────────────────────────────────────────────
        preview::show_window(ctx, self);

        // ── 关于窗口 ──────────────────────────────────────────────
        if self.about_open {
            let mut open = true;
            egui::Window::new(t("关于 FileSync", "About FileSync"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("FileSync");
                        ui.label(
                            egui::RichText::new(t(
                                "高性能文件夹同步工具",
                                "High-performance folder sync tool",
                            ))
                            .color(ui.visuals().weak_text_color()),
                        );
                        ui.add_space(12.0);
                        egui::Grid::new("about_grid")
                            .num_columns(2)
                            .spacing([16.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(t("版本", "Version"));
                                ui.label(env!("CARGO_PKG_VERSION"));
                                ui.end_row();
                                ui.label(t("构建工具链", "Toolchain"));
                                ui.label("Rust + egui 0.29");
                                ui.end_row();
                                ui.label(t("平台", "Platform"));
                                ui.label("Windows x86-64");
                                ui.end_row();
                                ui.label(t("配置路径", "Config path"));
                                let cfg = crate::config::storage::config_path()
                                    .to_string_lossy()
                                    .into_owned();
                                if ui.link(&cfg).clicked() {
                                    open_parent_in_explorer(&cfg);
                                }
                                ui.end_row();
                            });
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(t(
                                "支持 NTFS/ReFS 加速 · Delta 差量同步 · CopyFileEx",
                                "NTFS/ReFS USN acceleration · Delta sync · CopyFileEx",
                            ))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                        );
                    });
                    ui.add_space(8.0);
                });
            self.about_open = open;
        }

        self.show_error_dialog_window(ctx);
        self.show_unsaved_changes_dialog(ctx);

        // ── 应用内通知 ────────────────────────────────────────────
        show_notification_overlay(ctx, &mut self.notification);

        // ── 快捷键 ────────────────────────────────────────────────
        self.handle_shortcuts(ctx);
    }
}

// ─────────────────────────────────────────────────────────────────
// 辅助：在资源管理器中打开指定路径的父目录
// ─────────────────────────────────────────────────────────────────

fn open_parent_in_explorer(path: &str) {
    let p = std::path::Path::new(path);
    let dir = if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent()
            .map(|pp| pp.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
}

// ─────────────────────────────────────────────────────────────────
// 配置导出 / 导入
// ─────────────────────────────────────────────────────────────────

fn export_config(config: &AppConfig) {
    let json = match serde_json::to_string_pretty(config) {
        Ok(j) => j,
        Err(_) => return,
    };
    if let Some(path) = rfd::FileDialog::new()
        .set_title(t("导出配置", "Export Config"))
        .add_filter(t("JSON 配置", "JSON config"), &["json"])
        .set_file_name("filesync_config.json")
        .save_file()
    {
        let _ = std::fs::write(path, json);
    }
}

fn import_config() -> Option<AppConfig> {
    let path = rfd::FileDialog::new()
        .set_title(t("导入配置", "Import Config"))
        .add_filter(t("JSON 配置", "JSON config"), &["json"])
        .pick_file()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

// ─────────────────────────────────────────────────────────────────
// 应用内通知 overlay（底部右侧，4 秒后自动消失）
// ─────────────────────────────────────────────────────────────────

fn show_notification_overlay(
    ctx: &egui::Context,
    notif: &mut Option<AppNotification>,
) {
    let n = match notif {
        Some(n) => n,
        None => return,
    };

    let elapsed = n.created_at.elapsed().as_secs_f32();
    if elapsed >= 3.0 {
        *notif = None;
        return;
    }

    let remaining_secs = (3.0 - elapsed).ceil() as u32;

    let (icon, bg, accent) = match n.kind {
        NotificationKind::Success => (
            "✓",
            egui::Color32::from_rgb(25, 65, 25),
            egui::Color32::from_rgb(80, 200, 80),
        ),
        NotificationKind::Warning => (
            "⚠",
            egui::Color32::from_rgb(65, 55, 10),
            egui::Color32::from_rgb(220, 180, 40),
        ),
    };

    let title = format!("{} {}", icon, n.title);
    let body = n.body.clone();

    let mut should_dismiss = false;

    egui::Area::new("app_notification".into())
        .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(bg)
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                .stroke(egui::Stroke::new(1.0, accent))
                .show(ui, |ui| {
                    ui.set_max_width(280.0);

                    // 标题行 + 关闭按钮
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&title).color(accent).strong());
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add(egui::Label::new(
                                        egui::RichText::new("×").color(egui::Color32::GRAY),
                                    ).sense(egui::Sense::click()))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    should_dismiss = true;
                                }
                            },
                        );
                    });

                    // 详情文字
                    if !body.is_empty() {
                        ui.label(
                            egui::RichText::new(&body)
                                .small()
                                .color(egui::Color32::from_gray(200)),
                        );
                    }

                    // 倒计时秒数
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}s", remaining_secs))
                                .small()
                                .color(egui::Color32::from_gray(150)),
                        );
                    });
                });
        });

    if should_dismiss {
        *notif = None;
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

// ─────────────────────────────────────────────────────────────────
// 系统完成音效
// ─────────────────────────────────────────────────────────────────

fn play_completion_sound() {
    #[cfg(windows)]
    {
        // MB_ICONINFORMATION = 0x40，播放系统"通知"提示音
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBeep(utype: u32) -> i32;
        }
        unsafe { MessageBeep(0x40); }
    }
}

// ─────────────────────────────────────────────────────────────────
// 后台预览扫描（在普通 OS 线程中执行，无需 tokio）
// ─────────────────────────────────────────────────────────────────

fn run_preview_scan(
    job: crate::model::job::SyncJob,
) -> Result<Vec<PreviewEntry>, String> {
    use crate::engine::{diff, hash, scanner};
    use crate::engine::diff::DiffAction;

    let globset = scanner::build_globset(&job.exclusions);
    let mut all_entries = Vec::new();

    for pair in &job.folder_pairs {
        if !pair.enabled {
            continue;
        }
        if !pair.source.exists() {
            return Err(if is_zh() {
                format!("源目录不存在: {}", pair.source.display())
            } else {
                format!("Source directory not found: {}", pair.source.display())
            });
        }

        let src_scan = scanner::scan_directory(&pair.source, &globset).map_err(|e| {
            if is_zh() {
                format!("扫描源目录失败: {}", e)
            } else {
                format!("Failed to scan source directory: {}", e)
            }
        })?;

        let dst_scan = if pair.destination.exists() {
            scanner::scan_directory(&pair.destination, &globset)
                .unwrap_or_else(|_| scanner::ScanResult::empty())
        } else {
            scanner::ScanResult::empty()
        };

        let mut diffs =
            diff::compute_diff(&pair.source, &pair.destination, &src_scan, &dst_scan);

        // Hash 比对：精确排除内容相同的 Update
        if job.compare_method == CompareMethod::Hash {
            for d in diffs.iter_mut() {
                if d.action == DiffAction::Update {
                    if let (Some(sh), Some(dh)) =
                        (hash::hash_file(&d.source), hash::hash_file(&d.destination))
                    {
                        if sh == dh {
                            d.action = DiffAction::Skip;
                        }
                    }
                }
            }
        }

        for d in diffs {
            all_entries.push(PreviewEntry {
                relative_path: d.relative_path,
                action: d.action,
                size: d.size,
                modified: d.modified,
            });
        }

        // 孤立目录检测（源端不存在的目标端目录）
        for dir in crate::engine::executor::collect_orphan_dirs(&pair.source, &pair.destination) {
            let relative = dir
                .strip_prefix(&pair.destination)
                .map(|r| r.to_path_buf())
                .unwrap_or(dir);
            all_entries.push(PreviewEntry {
                relative_path: relative,
                action: DiffAction::Orphan,
                size: 0,
                modified: std::time::SystemTime::UNIX_EPOCH,
            });
        }
    }

    // 排序：Create/Update 在前，Skip 在后，Orphan 最后
    all_entries.sort_by_key(|e| match e.action {
        DiffAction::Create => 0u8,
        DiffAction::Update => 1,
        DiffAction::Skip => 2,
        DiffAction::Orphan => 3,
    });

    Ok(all_entries)
}

// ─────────────────────────────────────────────────────────────────
// 字体加载（CJK 支持）
// ─────────────────────────────────────────────────────────────────

/// 加载系统中文字体作为 egui 的 CJK 回退字形。
///
/// egui 默认只内置 Latin 字体，中文字符会渲染为方块。
/// 此函数依次尝试 Windows 内置的微软雅黑、黑体、宋体，
/// 将找到的第一个字体追加到字形回退链末尾。
fn setup_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑（Windows Vista+）
        r"C:\Windows\Fonts\simhei.ttf", // 黑体
        r"C:\Windows\Fonts\simsun.ttc", // 宋体（最旧的回退）
    ];

    let mut fonts = egui::FontDefinitions::default();

    for path in CANDIDATES {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk_fallback".to_owned(),
                egui::FontData::from_owned(data),
            );
            // 追加到回退链末尾：ASCII 仍由 egui 默认字体渲染，
            // 默认字体无法覆盖的 CJK 字符由此字体补充。
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("cjk_fallback".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk_fallback".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);

    // 全局字号：egui 默认 14pt，调大到 16pt 提升可读性
    let mut style = (*ctx.style()).clone();
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small,     FontId::proportional(12.0)),
        (TextStyle::Body,      FontId::proportional(16.0)),
        (TextStyle::Button,    FontId::proportional(16.0)),
        (TextStyle::Heading,   FontId::proportional(20.0)),
        (TextStyle::Monospace, FontId::monospace(15.0)),
    ]
    .into();
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::{
        build_completion_notification, completed_session_status, has_enabled_schedule,
        has_partial_enabled_folder_pair, has_valid_enabled_folder_pair, is_schedule_due,
        should_record_sync_completion, NotificationKind,
    };
    use crate::model::{
        config::AppConfig,
        job::{FolderPair, SyncJob, SyncSchedule},
        session::SessionStatus,
    };
    use chrono::{Duration, Utc};

    #[test]
    fn completed_session_status_tracks_stop_flag() {
        assert_eq!(completed_session_status(false), SessionStatus::Completed);
        assert_eq!(completed_session_status(true), SessionStatus::Stopped);
    }

    #[test]
    fn stopped_sync_does_not_record_completion_side_effects() {
        assert!(should_record_sync_completion(false));
        assert!(!should_record_sync_completion(true));
    }

    #[test]
    fn schedule_is_due_immediately_without_last_run() {
        assert!(is_schedule_due(None, 30, Utc::now()));
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
        let notification =
            build_completion_notification("Job A", 3, 2, 0, 0, false);

        assert_eq!(notification.title, "\"Job A\" sync complete");
        assert_eq!(notification.body, "Copied 3  Skipped 2");
        assert_eq!(notification.kind, NotificationKind::Success);
    }

    #[test]
    fn completion_notification_uses_warning_style_with_errors_and_deletes() {
        let notification =
            build_completion_notification("任务A", 5, 1, 2, 4, true);

        assert_eq!(notification.title, "「任务A」同步完成");
        assert_eq!(notification.body, "复制 5 个  跳过 1 个  错误 2 个  删除 4 个");
        assert_eq!(notification.kind, NotificationKind::Warning);
    }

    #[test]
    fn has_enabled_schedule_ignores_disabled_and_zero_interval_jobs() {
        let mut config = AppConfig::default();

        let mut disabled = SyncJob::new("disabled".into(), 1);
        disabled.schedule = SyncSchedule {
            enabled: false,
            interval_minutes: 30,
        };

        let mut zero_interval = SyncJob::new("zero".into(), 1);
        zero_interval.schedule = SyncSchedule {
            enabled: true,
            interval_minutes: 0,
        };

        let mut active = SyncJob::new("active".into(), 1);
        active.schedule = SyncSchedule {
            enabled: true,
            interval_minutes: 15,
        };

        config.jobs = vec![disabled, zero_interval];
        assert!(!has_enabled_schedule(&config));

        config.jobs.push(active);
        assert!(has_enabled_schedule(&config));
    }

    #[test]
    fn folder_pair_helpers_distinguish_partial_and_valid_pairs() {
        let mut partial = FolderPair::new();
        partial.source = "C:\\src".into();

        let mut valid = FolderPair::new();
        valid.source = "C:\\src".into();
        valid.destination = "D:\\dst".into();

        let disabled_empty = FolderPair {
            enabled: false,
            ..FolderPair::new()
        };

        assert!(has_partial_enabled_folder_pair(&[partial.clone()]));
        assert!(!has_valid_enabled_folder_pair(&[partial]));

        assert!(has_valid_enabled_folder_pair(&[valid.clone()]));
        assert!(!has_partial_enabled_folder_pair(&[valid]));

        assert!(!has_partial_enabled_folder_pair(&[disabled_empty.clone()]));
        assert!(!has_valid_enabled_folder_pair(&[disabled_empty]));
    }
}
