use super::*;

impl FileSyncApp {
    pub(super) fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let want_save = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::S));
        let want_sync = ctx.input(|i| i.key_pressed(egui::Key::F5));

        if want_save {
            self.save_selected_job_with_validation();
        }
        if want_sync && !self.sync_running {
            self.start_selected_sync_with_validation(ctx);
        }
    }

    pub(super) fn show_error_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.error_message.clone() else {
            return;
        };

        egui::Window::new("Error")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&msg);
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    self.error_message = None;
                }
            });
    }

    pub(super) fn show_delete_fallback_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = self.pending_delete_fallbacks.front() else {
            return;
        };

        let title = if request.is_dir {
            "Directory Delete Confirmation"
        } else {
            "File Delete Confirmation"
        };
        let body = format!(
            "Failed to move this item to the Recycle Bin:\n{}\n\n{}\n\nDo you want to continue with direct delete?",
            request.path.display(),
            request.message
        );

        let mut decision = None;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(body);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete Directly").clicked() {
                        decision = Some(DeleteFallbackChoice::DirectDelete);
                    }
                    if ui.button("Skip").clicked() {
                        decision = Some(DeleteFallbackChoice::Skip);
                    }
                    if ui.button("Stop Sync").clicked() {
                        decision = Some(DeleteFallbackChoice::StopSync);
                    }
                });
            });

        if let Some(choice) = decision {
            if let Some(request) = self.pending_delete_fallbacks.pop_front() {
                if choice == DeleteFallbackChoice::StopSync {
                    self.stop_sync();
                }
                let _ = request.response.send(choice);
            }
        }
    }

    pub(super) fn show_mass_delete_confirmation_dialog(&mut self, ctx: &egui::Context) {
        let Some(request) = &self.pending_mass_delete_confirmation else {
            return;
        };

        let mut proceed = None;
        let body = format!(
            "This mirror sync is about to delete {} destination items, exceeding the safety threshold.\n\nContinue?",
            request.count
        );

        egui::Window::new("Mass Delete Confirmation")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(body);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Continue").clicked() {
                        proceed = Some(true);
                    }
                    if ui.button("Cancel This Run").clicked() {
                        proceed = Some(false);
                    }
                });
            });

        if let Some(allow) = proceed {
            if let Some(request) = self.pending_mass_delete_confirmation.take() {
                let _ = request.response.send(allow);
            }
        }
    }

    pub(super) fn show_start_confirmation_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_start_confirmation else {
            return;
        };

        let Some(job_idx) = self.find_job_idx_by_id(pending.job_id) else {
            self.pending_start_confirmation = None;
            return;
        };
        let job = &self.config.jobs[job_idx];

        let mut confirmed = false;
        let mut cancelled = false;
        let mode_text = if job.sync_mode == SyncMode::Mirror {
            "Mirror sync deletes orphan files on destination."
        } else {
            ""
        };
        let delete_text = if matches!(job.delete_mode, crate::model::job::DeleteMode::Direct) {
            "Delete mode is direct delete, without Recycle Bin."
        } else {
            ""
        };

        egui::Window::new("Risk Confirmation")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(job.name.as_str());
                if !mode_text.is_empty() {
                    ui.label(
                        egui::RichText::new(mode_text)
                            .color(egui::Color32::from_rgb(255, 180, 80)),
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
                    if ui.button("Continue").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            if let Some(pending) = self.pending_start_confirmation.take() {
                if let Some(job_idx) = self.find_job_idx_by_id(pending.job_id) {
                    self.start_sync_entry(job_idx, pending.trigger, pending.retry_attempt, ctx);
                }
            }
        } else if cancelled {
            self.pending_start_confirmation = None;
        }
    }

    pub(super) fn show_history_window(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }

        let mut open = self.history_open;
        egui::Window::new("Task History")
            .open(&mut open)
            .resizable(true)
            .default_size([760.0, 520.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for job in &self.config.jobs {
                        let Some(state) = self.job_state(job.id) else {
                            continue;
                        };
                        if state.run_history.is_empty() {
                            continue;
                        }
                        ui.strong(job.name.as_str());
                        for entry in state.run_history.iter().take(20) {
                            let trigger = match entry.trigger {
                                RunTrigger::Manual => "Manual",
                                RunTrigger::Scheduled => "Scheduled",
                                RunTrigger::Retry => "Retry",
                            };
                            let result = match entry.result {
                                RunResultStatus::Completed => "Success",
                                RunResultStatus::Warning => "Warning",
                                RunResultStatus::Failed => "Failed",
                                RunResultStatus::Stopped => "Stopped",
                                RunResultStatus::Missed => "Missed",
                            };
                            let line = if let Some(summary) = &entry.summary {
                                format!(
                                    "{}  [{} / {}]  copied {}  skipped {}  errors {}  deleted {}  {}",
                                    entry
                                        .finished_at
                                        .with_timezone(&chrono::Local)
                                        .format("%m-%d %H:%M"),
                                    trigger,
                                    result,
                                    summary.copied,
                                    summary.skipped,
                                    summary.errors,
                                    summary.deleted,
                                    entry.note
                                )
                            } else {
                                format!(
                                    "{}  [{} / {}]  {}",
                                    entry
                                        .finished_at
                                        .with_timezone(&chrono::Local)
                                        .format("%m-%d %H:%M"),
                                    trigger,
                                    result,
                                    entry.note
                                )
                            };
                            ui.label(
                                egui::RichText::new(line)
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);
                    }
                });
            });
        self.history_open = open;
    }

    pub(super) fn show_unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        if !self.unsaved_dialog_open {
            return;
        }

        let mut keep_open = true;
        egui::Window::new("Unsaved Changes")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. Save before quitting?");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save();
                        self.unsaved_dialog_open = false;
                        self.quit_app_now();
                    }
                    if ui.button("Don't Save").clicked() {
                        self.unsaved_dialog_open = false;
                        self.quit_app_now();
                    }
                    if ui.button("Cancel").clicked() {
                        self.unsaved_dialog_open = false;
                    }
                });
            });

        if !keep_open {
            self.unsaved_dialog_open = false;
        }
    }
}
