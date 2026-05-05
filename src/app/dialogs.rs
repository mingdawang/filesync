use eframe::egui;

use crate::app::strings;
use crate::engine::events::DeleteFallbackChoice;
use crate::model::job::SyncMode;

use super::FileSyncApp;

pub(super) fn run_modal_windows(app: &mut FileSyncApp, ctx: &egui::Context) {
    if app.close_dialog_open {
        super::chrome::show_close_dialog(app, ctx);
    }
    show_delete_fallback_dialog(app, ctx);
    show_mass_delete_confirmation_dialog(app, ctx);
    show_start_confirmation_dialog(app, ctx);
    show_error_dialog_window(app, ctx);
    show_unsaved_changes_dialog(app, ctx);
}

pub(super) fn handle_shortcuts(app: &mut FileSyncApp, ctx: &egui::Context) {
    let want_save = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S));
    let want_sync = ctx.input(|i| i.key_pressed(egui::Key::F5));

    if want_save {
        app.save_selected_job_with_validation();
    }
    if want_sync && !app.sync_running {
        app.start_selected_sync_with_validation(ctx);
    }
}

fn show_error_dialog_window(app: &mut FileSyncApp, ctx: &egui::Context) {
    let Some(msg) = app.error_message.clone() else {
        return;
    };

    egui::Window::new(strings::error_title())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(&msg);
            ui.add_space(8.0);
            if ui.button(strings::ok_button()).clicked() {
                app.error_message = None;
            }
        });
}

fn show_delete_fallback_dialog(app: &mut FileSyncApp, ctx: &egui::Context) {
    let Some(request) = app.pending_delete_fallbacks.front() else {
        return;
    };

    let title = strings::delete_confirmation_title(request.is_dir);
    let body = strings::delete_fallback_body(&request.path.display().to_string(), &request.message);

    let mut decision = None;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(body);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(strings::delete_directly_button()).clicked() {
                    decision = Some(DeleteFallbackChoice::DirectDelete);
                }
                if ui.button(strings::skip_button()).clicked() {
                    decision = Some(DeleteFallbackChoice::Skip);
                }
                if ui.button(strings::stop_sync_button()).clicked() {
                    decision = Some(DeleteFallbackChoice::StopSync);
                }
            });
        });

    if let Some(choice) = decision {
        if let Some(request) = app.pending_delete_fallbacks.pop_front() {
            if choice == DeleteFallbackChoice::StopSync {
                app.stop_sync();
            }
            let _ = request.response.send(choice);
        }
    }
}

fn show_mass_delete_confirmation_dialog(app: &mut FileSyncApp, ctx: &egui::Context) {
    let Some(request) = &app.pending_mass_delete_confirmation else {
        return;
    };

    let mut proceed = None;
    let body = strings::mass_delete_body(request.count);

    egui::Window::new(strings::mass_delete_title())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(body);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(strings::continue_button()).clicked() {
                    proceed = Some(true);
                }
                if ui.button(strings::cancel_this_run_button()).clicked() {
                    proceed = Some(false);
                }
            });
        });

    if let Some(allow) = proceed {
        if let Some(request) = app.pending_mass_delete_confirmation.take() {
            let _ = request.response.send(allow);
        }
    }
}

fn show_start_confirmation_dialog(app: &mut FileSyncApp, ctx: &egui::Context) {
    let Some(pending) = &app.pending_start_confirmation else {
        return;
    };

    let Some(job_idx) = app.find_job_idx_by_id(pending.job_id) else {
        app.pending_start_confirmation = None;
        return;
    };
    let job = &app.config.jobs[job_idx];

    let mut confirmed = false;
    let mut cancelled = false;
    let mode_text = if job.sync_mode == SyncMode::Mirror {
        strings::mirror_delete_warning()
    } else {
        ""
    };
    let delete_text = if matches!(job.delete_mode, crate::model::job::DeleteMode::Direct) {
        strings::direct_delete_warning()
    } else {
        ""
    };

    egui::Window::new(strings::risk_confirmation_title())
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(job.name.as_str());
            if !mode_text.is_empty() {
                ui.label(
                    egui::RichText::new(mode_text).color(egui::Color32::from_rgb(255, 180, 80)),
                );
            }
            if !delete_text.is_empty() {
                ui.label(
                    egui::RichText::new(delete_text)
                        .color(egui::Color32::from_rgb(255, 120, 80)),
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(strings::continue_button()).clicked() {
                    confirmed = true;
                }
                if ui.button(strings::cancel_button()).clicked() {
                    cancelled = true;
                }
            });
        });

    if confirmed {
        if let Some(pending) = app.pending_start_confirmation.take() {
            if let Some(job_idx) = app.find_job_idx_by_id(pending.job_id) {
                app.start_sync_entry(job_idx, pending.trigger, pending.retry_attempt, ctx);
            }
        }
    } else if cancelled {
        app.pending_start_confirmation = None;
    }
}

fn show_unsaved_changes_dialog(app: &mut FileSyncApp, ctx: &egui::Context) {
    if !app.unsaved_dialog_open {
        return;
    }

    let mut keep_open = true;
    egui::Window::new(strings::unsaved_changes_title())
        .open(&mut keep_open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(strings::unsaved_changes_body());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(strings::save_button()).clicked() {
                    app.save();
                    app.unsaved_dialog_open = false;
                    app.quit_app_now();
                }
                if ui.button(strings::dont_save_button()).clicked() {
                    app.unsaved_dialog_open = false;
                    app.quit_app_now();
                }
                if ui.button(strings::cancel_button()).clicked() {
                    app.unsaved_dialog_open = false;
                }
            });
        });

    if !keep_open {
        app.unsaved_dialog_open = false;
    }
}
