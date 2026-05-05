use egui::Ui;

use crate::app::FileSyncApp;
use crate::i18n::{is_zh, t};
use crate::model::job::{RunResultStatus, SyncJob};
use crate::model::preview::PreviewState;

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
    ui.add_space(6.0);

    if ui.button(t("+ 新建任务", "+ New Job")).clicked() {
        let name = if is_zh() {
            format!("任务 {}", app.config.jobs.len() + 1)
        } else {
            format!("Job {}", app.config.jobs.len() + 1)
        };
        let concurrency = app.config.settings.default_concurrency;
        app.config.jobs.push(SyncJob::new(name, concurrency));
        app.selected_job = Some(app.config.jobs.len() - 1);
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    let mut to_delete = None;
    let mut to_duplicate = None;
    let mut to_move_up = None;
    let mut to_move_down = None;
    let job_count = app.config.jobs.len();
    let running_job_id = app.session.as_ref().map(|session| session.job_id);

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            for (i, job) in app.config.jobs.iter().enumerate() {
                let is_selected = app.selected_job == Some(i);
                let is_running = running_job_id == Some(job.id) && app.sync_running;
                let queued = app.job_queue.iter().position(|entry| entry.job_idx == i);

                let label = if is_selected {
                    egui::RichText::new(&job.name).strong()
                } else {
                    egui::RichText::new(&job.name)
                };
                let resp = ui.add_enabled(!app.sync_running, egui::SelectableLabel::new(is_selected, label));

                if resp.clicked() && !app.sync_running {
                    if app.selected_job != Some(i) {
                        app.preview_state = PreviewState::Idle;
                    }
                    app.selected_job = Some(i);
                    app.new_exclusion_input.clear();
                    app.exclusion_error = None;
                }

                resp.context_menu(|ui| {
                    if i > 0 && ui.button(t("上移", "Move Up")).clicked() {
                        to_move_up = Some(i);
                        ui.close_menu();
                    }
                    if i + 1 < job_count && ui.button(t("下移", "Move Down")).clicked() {
                        to_move_down = Some(i);
                        ui.close_menu();
                    }
                    if i > 0 || i + 1 < job_count {
                        ui.separator();
                    }
                    if ui.button(t("复制任务", "Duplicate Job")).clicked() {
                        to_duplicate = Some(i);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(t("删除任务", "Delete Job")).clicked() {
                        to_delete = Some(i);
                        ui.close_menu();
                    }
                });

                if is_running {
                    ui.label(
                        egui::RichText::new(t("  ● 运行中", "  ● Running"))
                            .small()
                            .color(egui::Color32::GREEN),
                    );
                } else if let Some(pos) = queued {
                    let text = if is_zh() {
                        format!("  ⏳ 队列 #{}", pos + 1)
                    } else {
                        format!("  ⏳ Queue #{}", pos + 1)
                    };
                    ui.label(
                        egui::RichText::new(text)
                            .small()
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                }

                if let Some(last) = job.last_sync_time {
                    let text = if is_zh() {
                        format!("  上次: {}", last.with_timezone(&chrono::Local).format("%m-%d %H:%M"))
                    } else {
                        format!("  Last: {}", last.with_timezone(&chrono::Local).format("%m-%d %H:%M"))
                    };
                    ui.label(egui::RichText::new(text).small().color(ui.visuals().weak_text_color()));
                }

                if job.schedule.enabled && job.schedule.interval_minutes > 0 {
                    let next_text = match job.last_sync_time {
                        Some(last) => {
                            let next = last + chrono::Duration::minutes(job.schedule.interval_minutes as i64);
                            if next <= chrono::Utc::now() {
                                t("  ⏰ 待调度", "  ⏰ Due").to_string()
                            } else if is_zh() {
                                format!("  ⏰ 下次: {}", next.with_timezone(&chrono::Local).format("%m-%d %H:%M"))
                            } else {
                                format!("  ⏰ Next: {}", next.with_timezone(&chrono::Local).format("%m-%d %H:%M"))
                            }
                        }
                        None => t("  ⏰ 首次待运行", "  ⏰ First run pending").to_string(),
                    };
                    ui.label(
                        egui::RichText::new(next_text)
                            .small()
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                }

                if let Some(summary) = &job.last_run_summary {
                    let text = if is_zh() {
                        format!(
                            "  复制 {}  跳过 {}  错误 {}  删除 {}",
                            summary.copied, summary.skipped, summary.errors, summary.deleted
                        )
                    } else {
                        format!(
                            "  Copied {}  Skipped {}  Errors {}  Deleted {}",
                            summary.copied, summary.skipped, summary.errors, summary.deleted
                        )
                    };
                    ui.label(
                        egui::RichText::new(text)
                            .small()
                            .color(if summary.errors > 0 {
                                egui::Color32::from_rgb(255, 160, 50)
                            } else {
                                ui.visuals().weak_text_color()
                            }),
                    );
                }

                if let Some(entry) = job.run_history.first() {
                    let (text, color) = match entry.result {
                        RunResultStatus::Completed => (
                            t("  最近: 成功", "  Recent: Success"),
                            egui::Color32::from_rgb(90, 200, 120),
                        ),
                        RunResultStatus::Warning => (
                            t("  最近: 有错误", "  Recent: Warning"),
                            egui::Color32::from_rgb(255, 180, 80),
                        ),
                        RunResultStatus::Failed => (
                            t("  最近: 失败", "  Recent: Failed"),
                            egui::Color32::RED,
                        ),
                        RunResultStatus::Stopped => (
                            t("  最近: 已停止", "  Recent: Stopped"),
                            egui::Color32::GRAY,
                        ),
                        RunResultStatus::Missed => (
                            t("  最近: 漏跑", "  Recent: Missed"),
                            egui::Color32::from_rgb(180, 120, 80),
                        ),
                    };
                    ui.label(egui::RichText::new(text).small().color(color));
                }

                ui.add_space(6.0);
            }
        });
    });

    if let Some(i) = to_move_up {
        app.config.jobs.swap(i, i - 1);
        if app.selected_job == Some(i) {
            app.selected_job = Some(i - 1);
        } else if app.selected_job == Some(i - 1) {
            app.selected_job = Some(i);
        }
        app.config.jobs[i].dirty = true;
        app.config.jobs[i - 1].dirty = true;
    } else if let Some(i) = to_move_down {
        app.config.jobs.swap(i, i + 1);
        if app.selected_job == Some(i) {
            app.selected_job = Some(i + 1);
        } else if app.selected_job == Some(i + 1) {
            app.selected_job = Some(i);
        }
        app.config.jobs[i].dirty = true;
        app.config.jobs[i + 1].dirty = true;
    }

    if let Some(i) = to_duplicate {
        let mut new_job = app.config.jobs[i].clone();
        new_job.id = uuid::Uuid::new_v4();
        new_job.name = if is_zh() {
            format!("{} (副本)", new_job.name)
        } else {
            format!("{} (Copy)", new_job.name)
        };
        new_job.last_sync_time = None;
        new_job.last_run_summary = None;
        new_job.run_history.clear();
        new_job.schedule.enabled = false;
        new_job.dirty = true;
        app.config.jobs.insert(i + 1, new_job);
        app.selected_job = Some(i + 1);
    }

    if let Some(idx) = to_delete {
        app.pending_delete = Some(idx);
    }

    if let Some(idx) = app.pending_delete {
        let job_name = app
            .config
            .jobs
            .get(idx)
            .map(|j| j.name.clone())
            .unwrap_or_default();

        let mut keep_open = true;
        egui::Window::new(t("确认删除", "Confirm Delete"))
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                let msg = if is_zh() {
                    format!("确定要删除任务“{}”吗？此操作不可撤销。", job_name)
                } else {
                    format!("Delete job \"{}\"? This cannot be undone.", job_name)
                };
                ui.label(msg);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(t("删除", "Delete")).clicked() {
                        app.config.jobs.remove(idx);
                        app.selected_job = if app.config.jobs.is_empty() {
                            None
                        } else {
                            Some(idx.saturating_sub(1).min(app.config.jobs.len() - 1))
                        };
                        if !app.is_dirty() {
                            app.save();
                        }
                        app.pending_delete = None;
                    }
                    if ui.button(t("取消", "Cancel")).clicked() {
                        app.pending_delete = None;
                    }
                });
            });

        if !keep_open {
            app.pending_delete = None;
        }
    }
}
