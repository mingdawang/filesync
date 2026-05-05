use eframe::egui;

use crate::app::{config_io, strings, support, CloseDialogAction, FileSyncApp};
use crate::log::LogLevel;
use crate::model::config::{CloseAction, Theme};
use crate::ui::{job_editor, job_list, progress};

pub(super) fn apply_theme(app: &FileSyncApp, ctx: &egui::Context) {
    match app.config.settings.theme {
        Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
        Theme::Light => ctx.set_visuals(egui::Visuals::light()),
        Theme::System => {}
    }
}

pub(super) fn handle_close_requests(app: &mut FileSyncApp, ctx: &egui::Context) {
    if crate::tray::close_button_clicked() {
        crate::tray::reset_close_button();
        handle_close_button_click(app);
    }

    if force_quit_requested(app) {
        app.quit_app();
    }

    if ctx.input(|i| i.viewport().close_requested()) {
        app.quit_app();
    }
}

pub(super) fn show_close_dialog(app: &mut FileSyncApp, ctx: &egui::Context) {
    let mut remember = app.close_dialog_remember;
    let mut action: Option<CloseDialogAction> = None;

    egui::Window::new(strings::close_title())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);
            if app.sync_running {
                ui.label(
                    egui::RichText::new(strings::close_running_warning())
                        .color(egui::Color32::from_rgb(255, 180, 50)),
                );
                ui.add_space(8.0);
            }
            ui.label(strings::choose_close_action());
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                if ui.button(strings::minimize_to_tray_button()).clicked() {
                    action = Some(CloseDialogAction::Minimize);
                }
                ui.add_space(8.0);
                if ui.button(strings::quit_button()).clicked() {
                    action = Some(CloseDialogAction::Quit);
                }
                ui.add_space(8.0);
                if ui.button(strings::cancel_button()).clicked() {
                    action = Some(CloseDialogAction::Cancel);
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            ui.checkbox(&mut remember, strings::remember_close_choice());
        });

    app.close_dialog_remember = remember;

    match action {
        Some(CloseDialogAction::Minimize) => {
            app.close_dialog_open = false;
            if app.tray.is_some() {
                if app.close_dialog_remember {
                    app.config.settings.close_action = CloseAction::MinimizeToTray;
                    app.settings_dirty = true;
                    app.save();
                }
                crate::tray::hide_app_window();
            } else {
                crate::log::app_log(
                    "close dialog: minimize requested but no tray, quitting instead",
                    LogLevel::Info,
                );
                app.quit_app();
            }
        }
        Some(CloseDialogAction::Quit) => {
            app.close_dialog_open = false;
            if app.close_dialog_remember {
                app.config.settings.close_action = CloseAction::Quit;
                app.settings_dirty = true;
            }
            app.quit_app();
        }
        Some(CloseDialogAction::Cancel) => {
            app.close_dialog_open = false;
            app.close_dialog_remember = false;
        }
        None => {}
    }
}

pub(super) fn render_top_panel(app: &mut FileSyncApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("FileSync");
            ui.separator();
            if app.is_dirty() {
                ui.label(
                    egui::RichText::new(strings::top_unsaved_changes())
                        .color(egui::Color32::from_rgb(255, 180, 80))
                        .small(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(strings::about_button()).clicked() {
                    app.about_open = !app.about_open;
                }
                if ui.small_button(strings::settings_button()).clicked() {
                    app.settings_open = !app.settings_open;
                }
                if ui.small_button(strings::history_button()).clicked() {
                    app.history_open = !app.history_open;
                }
                ui.label(
                    egui::RichText::new(strings::shortcuts_hint())
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
    });
}

pub(super) fn render_progress_panel(app: &mut FileSyncApp, ctx: &egui::Context) {
    let progress_default_h = app
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
                    progress::show(ui, app);
                });
        });
    app.progress_panel_height = Some(progress_panel.response.rect.height());
}

pub(super) fn render_job_list_panel(app: &mut FileSyncApp, ctx: &egui::Context) {
    egui::SidePanel::left("job_list_panel")
        .resizable(false)
        .exact_width(210.0)
        .show(ctx, |ui| {
            job_list::show(ui, app);
        });
}

pub(super) fn render_main_panel(app: &mut FileSyncApp, ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        job_editor::show(ui, app);
    });
}

pub(super) fn render_settings_window(app: &mut FileSyncApp, ctx: &egui::Context) {
    if !app.settings_open {
        return;
    }

    let mut open = app.settings_open;
    egui::Window::new(strings::settings_title())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .min_width(280.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.strong(strings::theme_title());
            ui.add_space(4.0);
            let theme_options: &[(&str, Theme)] = &[
                (strings::follow_system_theme(), Theme::System),
                (strings::light_theme(), Theme::Light),
                (strings::dark_theme(), Theme::Dark),
            ];
            for (label, variant) in theme_options {
                if ui.radio(app.config.settings.theme == *variant, *label).clicked() {
                    app.config.settings.theme = variant.clone();
                    app.settings_dirty = true;
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.strong(strings::default_concurrency_title());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut c = app.config.settings.default_concurrency;
                if ui
                    .add(egui::Slider::new(&mut c, 1usize..=16).text(strings::threads_label()))
                    .changed()
                {
                    app.config.settings.default_concurrency = c;
                    app.settings_dirty = true;
                }
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            if app.tray.is_some() {
                ui.strong(strings::close_action_title());
                ui.add_space(4.0);
                let close_options: &[(&str, CloseAction)] = &[
                    (strings::ask_every_time(), CloseAction::Ask),
                    (strings::minimize_to_tray_button(), CloseAction::MinimizeToTray),
                    (strings::quit_button(), CloseAction::Quit),
                ];
                for (label, variant) in close_options {
                    if ui
                        .radio(app.config.settings.close_action == *variant, *label)
                        .clicked()
                    {
                        app.config.settings.close_action = variant.clone();
                        app.settings_dirty = true;
                    }
                }
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
            }

            ui.strong(strings::config_backup_title());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(strings::export_config_button()).clicked() {
                    config_io::export_config(&app.config);
                }
                ui.add_space(8.0);
                if ui.button(strings::import_config_button()).clicked() {
                    if let Some(imported) = config_io::import_config() {
                        app.config = imported;
                        app.settings_dirty = false;
                        app.job_transient.clear();
                        for job in &app.config.jobs {
                            app.job_transient.entry(job.id).or_default();
                        }
                        if let Some(sel) = app.selected_job {
                            if sel >= app.config.jobs.len() {
                                app.selected_job = None;
                            }
                        }
                    }
                }
            });
            ui.label(
                egui::RichText::new(strings::import_overwrite_warning())
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button(strings::save_button()).clicked() {
                    app.save();
                }
            });
        });
    app.settings_open = open;
}

pub(super) fn render_about_window(app: &mut FileSyncApp, ctx: &egui::Context) {
    if !app.about_open {
        return;
    }

    let mut open = true;
    egui::Window::new(strings::about_title())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.heading("FileSync");
                ui.label(
                    egui::RichText::new(strings::about_subtitle())
                        .color(ui.visuals().weak_text_color()),
                );
                ui.add_space(12.0);
                egui::Grid::new("about_grid")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(strings::version_label());
                        ui.label(env!("CARGO_PKG_VERSION"));
                        ui.end_row();
                        ui.label(strings::toolchain_label());
                        ui.label("Rust + egui 0.29");
                        ui.end_row();
                        ui.label(strings::platform_label());
                        ui.label("Windows x86-64");
                        ui.end_row();
                        ui.label(strings::config_path_label());
                        let cfg = crate::config::storage::config_path()
                            .to_string_lossy()
                            .into_owned();
                        if ui.link(&cfg).clicked() {
                            support::open_parent_in_explorer(&cfg);
                        }
                        ui.end_row();
                    });
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(strings::about_features())
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            });
            ui.add_space(8.0);
        });
    app.about_open = open;
}

fn handle_close_button_click(app: &mut FileSyncApp) {
    if app.close_dialog_open {
        return;
    }

    match &app.config.settings.close_action {
        CloseAction::MinimizeToTray if app.tray.is_some() => {
            crate::tray::hide_app_window();
        }
        CloseAction::Ask if app.tray.is_some() => {
            app.close_dialog_open = true;
        }
        _ => app.quit_app(),
    }
}

fn force_quit_requested(app: &FileSyncApp) -> bool {
    app.tray
        .as_ref()
        .is_some_and(|t| t.force_quit.load(std::sync::atomic::Ordering::Acquire))
}
