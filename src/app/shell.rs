use eframe::egui;

use crate::app::{chrome, dialogs, support, FileSyncApp};
use crate::log::LogLevel;
use crate::ui::preview;

impl eframe::App for FileSyncApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        crate::log::app_log("on_exit() called", LogLevel::Info);
        self.tray = None;
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        static FIRST_UPDATE: std::sync::atomic::AtomicBool =
            std::sync::atomic::AtomicBool::new(true);
        if FIRST_UPDATE.swap(false, std::sync::atomic::Ordering::Relaxed) {
            crate::log::app_log("update() first frame", LogLevel::Info);
        }

        crate::tray::install_close_hook_once();

        chrome::handle_close_requests(self, ctx);
        chrome::apply_theme(self, ctx);
        self.drain_events();
        self.drain_preview();

        self.start_pending_queued_job(ctx);
        self.trigger_scheduled_sync_if_due(ctx);

        if self.sync_running {
            ctx.request_repaint();
        } else {
            self.request_schedule_wake_if_needed(ctx);
        }

        chrome::render_top_panel(self, ctx);
        chrome::render_progress_panel(self, ctx);
        chrome::render_job_list_panel(self, ctx);
        chrome::render_main_panel(self, ctx);
        chrome::render_settings_window(self, ctx);

        preview::show_window(ctx, self);
        dialogs::run_modal_windows(self, ctx);
        chrome::render_about_window(self, ctx);

        support::show_notification_overlay(ctx, &mut self.notification);
        dialogs::handle_shortcuts(self, ctx);
    }
}
