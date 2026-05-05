use super::*;

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

        self.handle_close_requests(ctx);

        if self.close_dialog_open {
            self.show_close_dialog(ctx);
        }
        self.show_delete_fallback_dialog(ctx);
        self.show_mass_delete_confirmation_dialog(ctx);
        self.show_start_confirmation_dialog(ctx);

        self.apply_theme(ctx);
        self.drain_events();
        self.drain_preview();

        self.start_pending_queued_job(ctx);
        self.trigger_scheduled_sync_if_due(ctx);

        if self.sync_running {
            ctx.request_repaint();
        } else {
            self.request_schedule_wake_if_needed(ctx);
        }

        self.render_top_panel(ctx);
        self.render_progress_panel(ctx);
        self.render_job_list_panel(ctx);
        self.render_main_panel(ctx);
        self.render_settings_window(ctx);

        preview::show_window(ctx, self);
        self.show_history_window(ctx);
        self.render_about_window(ctx);

        self.show_error_dialog_window(ctx);
        self.show_unsaved_changes_dialog(ctx);
        show_notification_overlay(ctx, &mut self.notification);
        self.handle_shortcuts(ctx);
    }
}

impl FileSyncApp {
    pub(super) fn apply_theme(&self, ctx: &egui::Context) {
        match self.config.settings.theme {
            Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
            Theme::Light => ctx.set_visuals(egui::Visuals::light()),
            Theme::System => {}
        }
    }

    pub(super) fn handle_close_requests(&mut self, ctx: &egui::Context) {
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
            .is_some_and(|t| t.force_quit.load(std::sync::atomic::Ordering::Acquire))
    }

    pub(super) fn show_close_dialog(&mut self, ctx: &egui::Context) {
        let mut remember = self.close_dialog_remember;
        let mut action: Option<CloseDialogAction> = None;

        egui::Window::new("Close FileSync")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                if self.sync_running {
                    ui.label(
                        egui::RichText::new(
                            "Sync is in progress. Quitting will interrupt it.",
                        )
                        .color(egui::Color32::from_rgb(255, 180, 50)),
                    );
                    ui.add_space(8.0);
                }
                ui.label("Choose what to do:");
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("Minimize to Tray").clicked() {
                        action = Some(CloseDialogAction::Minimize);
                    }
                    ui.add_space(8.0);
                    if ui.button("Quit").clicked() {
                        action = Some(CloseDialogAction::Quit);
                    }
                    ui.add_space(8.0);
                    if ui.button("Cancel").clicked() {
                        action = Some(CloseDialogAction::Cancel);
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                ui.checkbox(
                    &mut remember,
                    "Remember my choice (can be changed in Settings)",
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

    fn render_top_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("FileSync");
                ui.separator();
                if self.is_dirty() {
                    ui.label(
                        egui::RichText::new("Unsaved changes")
                            .color(egui::Color32::from_rgb(255, 180, 80))
                            .small(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("About").clicked() {
                        self.about_open = !self.about_open;
                    }
                    if ui.small_button("Settings").clicked() {
                        self.settings_open = !self.settings_open;
                    }
                    if ui.small_button("History").clicked() {
                        self.history_open = !self.history_open;
                    }
                    ui.label(
                        egui::RichText::new("Ctrl+S Save  F5 Sync")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            });
        });
    }

    fn render_progress_panel(&mut self, ctx: &egui::Context) {
        let progress_default_h = self
            .progress_panel_height
            .unwrap_or_else(|| (ctx.screen_rect().height() * 0.40).clamp(220.0, 420.0));
        let progress_panel = egui::TopBottomPanel::bottom("progress_panel")
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
        self.progress_panel_height = Some(progress_panel.response.rect.height());
    }

    fn render_job_list_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("job_list_panel")
            .resizable(false)
            .exact_width(210.0)
            .show(ctx, |ui| {
                job_list::show(ui, self);
            });
    }

    fn render_main_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            job_editor::show(ui, self);
        });
    }

    fn render_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }

        let mut open = self.settings_open;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(280.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.strong("Theme");
                ui.add_space(4.0);
                let theme_options: &[(&str, Theme)] = &[
                    ("Follow System", Theme::System),
                    ("Light", Theme::Light),
                    ("Dark", Theme::Dark),
                ];
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

                ui.strong("Default Concurrency for New Jobs");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let mut c = self.config.settings.default_concurrency;
                    if ui
                        .add(egui::Slider::new(&mut c, 1usize..=16).text("threads"))
                        .changed()
                    {
                        self.config.settings.default_concurrency = c;
                        self.settings_dirty = true;
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                if self.tray.is_some() {
                    ui.strong("When clicking Close (X)");
                    ui.add_space(4.0);
                    let close_options: &[(&str, crate::model::config::CloseAction)] = &[
                        ("Ask every time", crate::model::config::CloseAction::Ask),
                        (
                            "Minimize to tray",
                            crate::model::config::CloseAction::MinimizeToTray,
                        ),
                        ("Quit", crate::model::config::CloseAction::Quit),
                    ];
                    for (label, variant) in close_options {
                        if ui
                            .radio(self.config.settings.close_action == *variant, *label)
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

                ui.strong("Config Backup");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Export Config").clicked() {
                        export_config(&self.config);
                    }
                    ui.add_space(8.0);
                    if ui.button("Import Config").clicked() {
                        if let Some(imported) = import_config() {
                            self.config = imported;
                            self.settings_dirty = false;
                            self.selected_job = None;
                        }
                    }
                });
                ui.label(
                    egui::RichText::new("Import will overwrite all current jobs and settings")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save();
                    }
                });
            });
        self.settings_open = open;
    }

    fn render_about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }

        let mut open = true;
        egui::Window::new("About FileSync")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.heading("FileSync");
                    ui.label(
                        egui::RichText::new("High-performance folder sync tool")
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(12.0);
                    egui::Grid::new("about_grid")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Version");
                            ui.label(env!("CARGO_PKG_VERSION"));
                            ui.end_row();
                            ui.label("Toolchain");
                            ui.label("Rust + egui 0.29");
                            ui.end_row();
                            ui.label("Platform");
                            ui.label("Windows x86-64");
                            ui.end_row();
                            ui.label("Config path");
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
                        egui::RichText::new(
                            "NTFS/ReFS USN acceleration, Delta sync, and CopyFileEx support",
                        )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                });
                ui.add_space(8.0);
            });
        self.about_open = open;
    }
}
