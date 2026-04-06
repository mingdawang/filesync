use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;

use crate::config::storage;
use crate::engine::events::SyncEvent;
use crate::i18n::{is_zh, t};
use crate::model::config::{AppConfig, CompareMethod, Theme};
use crate::model::preview::{PreviewEntry, PreviewState};
use crate::model::session::{ErrorKind, SessionStatus, SyncError, SyncSession, WorkerState};
use crate::ui::{job_editor, job_list, preview, progress};

use crate::log::LogLevel;

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
        let Some(job) = self.config.jobs.get(idx) else {
            return None;
        };
        let has_partial = job.folder_pairs.iter().any(|p| {
            p.enabled
                && (p.source.as_os_str().is_empty() != p.destination.as_os_str().is_empty())
        });
        if has_partial {
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
        let Some(job) = self.config.jobs.get(idx) else {
            return None;
        };
        let has_partial = job.folder_pairs.iter().any(|p| {
            p.enabled
                && (p.source.as_os_str().is_empty() != p.destination.as_os_str().is_empty())
        });
        if has_partial {
            return Some(
                t(
                    "存在已启用但源/目标路径不完整的文件夹对，请检查配置。",
                    "Some enabled folder pairs have incomplete paths. Please fix them.",
                )
                .into(),
            );
        }
        let has_valid = job.folder_pairs.iter().any(|p| {
            p.enabled
                && !p.source.as_os_str().is_empty()
                && !p.destination.as_os_str().is_empty()
        });
        if !has_valid {
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
                    crate::log::app_log(
                        &format!("failed to create tokio runtime: {}", e),
                        LogLevel::Error,
                    );
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
        let Some(session) = &mut self.session else { return };

        match event {
            SyncEvent::Started { total_files, total_bytes } => {
                session.stats.total_files = total_files;
                session.stats.total_bytes = total_bytes;
                session.status = SessionStatus::Running;
            }

            SyncEvent::FileStarted { worker_id, path, size, is_new } => {
                if worker_id < session.active_workers.len() {
                    session.active_workers[worker_id] = WorkerState::Copying {
                        path,
                        size,
                        done: 0,
                        is_new,
                    };
                }
            }

            SyncEvent::FileProgress { worker_id, bytes_done } => {
                if worker_id < session.active_workers.len() {
                    if let WorkerState::Copying { done, .. } =
                        &mut session.active_workers[worker_id]
                    {
                        *done = bytes_done;
                    }
                }
            }

            SyncEvent::FileCompleted { worker_id, path, size, delta, saved_bytes, .. } => {
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
                session.copied_log.push(crate::model::session::CopiedFileEntry {
                    path: path.clone(),
                    size,
                    delta,
                });
            }

            SyncEvent::FileSkipped { .. } => {
                session.stats.skipped_files += 1;
                session.stats.processed_files += 1;
            }

            SyncEvent::FileDeleted { path } => {
                session.stats.deleted_files += 1;
                session.deleted_paths.push(path);
            }

            SyncEvent::FileOrphan { path } => {
                session.orphan_log.push(path);
            }

            SyncEvent::FileError { path, message } => {
                session.stats.error_count += 1;
                session.stats.processed_files += 1;
                session.errors.push(SyncError {
                    timestamp: Utc::now(),
                    path,
                    kind: ErrorKind::IoError,
                    message,
                });
            }

            SyncEvent::Completed { stats, usn_checkpoints } => {
                let elapsed_secs =
                    (Utc::now() - session.started_at).num_seconds().max(0) as u64;
                let summary = crate::model::job::RunSummary {
                    copied: stats.copied_files,
                    skipped: stats.skipped_files,
                    errors: stats.error_count,
                    deleted: stats.deleted_files,
                    bytes: stats.copied_bytes,
                    elapsed_secs,
                };
                // 为通知保存副本（summary 稍后会被 move）
                let n_copied = summary.copied;
                let n_skipped = summary.skipped;
                let n_errors = summary.errors;
                let n_deleted = summary.deleted;

                // 记录刚完成的任务名（队列推进前）
                let finished_job_name = self
                    .selected_job
                    .and_then(|i| self.config.jobs.get(i))
                    .map(|j| j.name.clone())
                    .unwrap_or_default();

                session.stats = stats;
                session.status = SessionStatus::Completed;
                for w in &mut session.active_workers {
                    *w = WorkerState::Idle;
                }

                // 写同步日志文件
                {
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
                        crate::log::app_log(&format!("write_sync_log failed: {}", e), crate::log::LogLevel::Error);
                    }
                }

                self.sync_running = false;
                play_completion_sound();
                // 队列中还有任务时自动启动下一个
                // （注意：此处不能借用 ctx，dequeue 只标记下一个 idx，
                //  update() 会在下一帧检查并启动）
                if let Some(next_idx) = self.job_queue.pop_front() {
                    self.selected_job = Some(next_idx);
                    self.pending_queue_start = true;
                }
                if let Some(idx) = self.selected_job {
                    if let Some(job) = self.config.jobs.get_mut(idx) {
                        job.last_sync_time = Some(Utc::now());
                        job.last_run_summary = Some(summary);
                        // 保存 USN 检查点（仅无错误的完整同步）
                        if !usn_checkpoints.is_empty() {
                            for (vol, (journal_id, next_usn)) in usn_checkpoints {
                                job.last_sync_checkpoints.insert(
                                    vol,
                                    crate::model::job::UsnCheckpoint { journal_id, next_usn },
                                );
                            }
                        }
                        // 运行统计自动保存，不标记 dirty（用户未修改配置）
                        if let Err(e) = crate::config::storage::save(&self.config) {
                            crate::log::app_log(
                                &format!("auto-save after sync failed (USN checkpoints may be lost): {}", e),
                                LogLevel::Error,
                            );
                        }
                    }
                }

                // 应用内完成通知
                let mut body_parts = if is_zh() {
                    vec![
                        format!("复制 {} 个", n_copied),
                        format!("跳过 {} 个", n_skipped),
                    ]
                } else {
                    vec![
                        format!("Copied {}", n_copied),
                        format!("Skipped {}", n_skipped),
                    ]
                };
                if n_errors > 0 {
                    body_parts.push(if is_zh() {
                        format!("错误 {} 个", n_errors)
                    } else {
                        format!("Errors {}", n_errors)
                    });
                }
                if n_deleted > 0 {
                    body_parts.push(if is_zh() {
                        format!("删除 {} 个", n_deleted)
                    } else {
                        format!("Deleted {}", n_deleted)
                    });
                }
                self.notification = Some(AppNotification {
                    title: if is_zh() {
                        format!("「{}」同步完成", finished_job_name)
                    } else {
                        format!("\"{}\" sync complete", finished_job_name)
                    },
                    body: body_parts.join("  "),
                    created_at: std::time::Instant::now(),
                    kind: if n_errors > 0 {
                        NotificationKind::Warning
                    } else {
                        NotificationKind::Success
                    },
                });
            }

            SyncEvent::DiskFull => {
                session.status = SessionStatus::Failed;
                self.sync_running = false;
                self.error_message = Some(
                    t("磁盘空间不足，同步已停止！", "Disk full — sync stopped!").into(),
                );
            }

            SyncEvent::Paused => session.status = SessionStatus::Paused,
            SyncEvent::Resumed => session.status = SessionStatus::Running,
            SyncEvent::SpeedUpdate { bps } => session.stats.speed_bps = bps,
        }
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
            let interval =
                chrono::Duration::minutes(job.schedule.interval_minutes as i64);
            let due = match job.last_sync_time {
                Some(t) => now >= t + interval,
                None => true, // 从未同步过，立即执行
            };
            if due {
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

        // 路径 A
        if crate::tray::close_button_clicked() {
            crate::tray::reset_close_button();
            use crate::model::config::CloseAction;
            if !self.close_dialog_open {
                match &self.config.settings.close_action {
                    CloseAction::MinimizeToTray if self.tray.is_some() => {
                        crate::tray::hide_app_window();
                    }
                    CloseAction::Ask if self.tray.is_some() => {
                        self.close_dialog_open = true;
                    }
                    _ => {
                        self.quit_app();
                    }
                }
            }
        }

        // 路径 B
        let force_quit = self
            .tray
            .as_ref()
            .map_or(false, |t| t.force_quit.load(std::sync::atomic::Ordering::Acquire));
        if force_quit {
            self.quit_app();
        }

        // 路径 C（保底）
        if ctx.input(|i| i.viewport().close_requested()) {
            self.quit_app();
        }

        // ── 关闭确认对话框 ────────────────────────────────────────
        if self.close_dialog_open {
            let mut do_minimize = false;
            let mut do_quit = false;
            let mut do_cancel = false;
            let mut remember = self.close_dialog_remember;

            egui::Window::new(t("关闭 FileSync", "Close FileSync"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    if self.sync_running {
                        ui.label(
                            egui::RichText::new(t(
                                "⚠ 同步正在进行中，退出将中断当前同步。",
                                "⚠ Sync is in progress. Quitting will interrupt it.",
                            ))
                            .color(egui::Color32::from_rgb(255, 180, 50)),
                        );
                        ui.add_space(8.0);
                    }
                    ui.label(t("请选择关闭行为：", "Choose what to do:"));
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button(t("最小化到托盘", "Minimize to Tray")).clicked() {
                            do_minimize = true;
                        }
                        ui.add_space(8.0);
                        if ui.button(t("退出程序", "Quit")).clicked() {
                            do_quit = true;
                        }
                        ui.add_space(8.0);
                        if ui.button(t("取消", "Cancel")).clicked() {
                            do_cancel = true;
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

            if do_minimize {
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
                    // No tray available (e.g. Safe Mode), fall back to quit
                    crate::log::app_log("close dialog: minimize requested but no tray, quitting instead", LogLevel::Info);
                    self.quit_app();
                }
            } else if do_quit {
                self.close_dialog_open = false;
                if self.close_dialog_remember {
                    self.config.settings.close_action =
                        crate::model::config::CloseAction::Quit;
                    self.settings_dirty = true;
                }
                self.quit_app();
            } else if do_cancel {
                self.close_dialog_open = false;
                self.close_dialog_remember = false;
            }
        }

        self.apply_theme(ctx);
        self.drain_events();
        self.drain_preview();

        // 队列自动启动下一个任务
        if self.pending_queue_start && !self.sync_running {
            self.pending_queue_start = false;
            self.start_sync(ctx);
        }

        // 定时同步检查
        if let Some(idx) = self.check_scheduled_sync() {
            self.selected_job = Some(idx);
            self.save();
            self.start_sync(ctx);
        }

        if self.sync_running {
            ctx.request_repaint();
        } else {
            // 有启用的定时任务时，每 30 秒唤醒一次以检查到期
            let has_schedule = self
                .config
                .jobs
                .iter()
                .any(|j| j.schedule.enabled && j.schedule.interval_minutes > 0);
            if has_schedule {
                ctx.request_repaint_after(std::time::Duration::from_secs(30));
            }
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

        // ── 全局错误弹窗 ──────────────────────────────────────────
        if let Some(msg) = self.error_message.clone() {
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

        // ── 未保存修改确认弹窗 ──────────────────────────────────────
        if self.unsaved_dialog_open {
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

        // ── 应用内通知 ────────────────────────────────────────────
        show_notification_overlay(ctx, &mut self.notification);

        // ── 快捷键 ────────────────────────────────────────────────
        let want_save = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S));
        let want_sync = ctx.input(|i| i.key_pressed(egui::Key::F5));

        if want_save {
            if let Some(idx) = self.selected_job {
                if let Some(err) = self.validate_folder_pairs_for_save(idx) {
                    self.error_message = Some(err);
                    return;
                }
            }
            self.save();
        }
        if want_sync && !self.sync_running {
            if let Some(idx) = self.selected_job {
                if let Some(err) = self.validate_folder_pairs_for_start(idx) {
                    self.error_message = Some(err);
                    return;
                }
            }
            self.start_sync(ctx);
        }
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
