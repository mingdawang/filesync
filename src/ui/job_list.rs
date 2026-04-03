use egui::Ui;

use crate::app::FileSyncApp;
use crate::i18n::{is_zh, t};
use crate::model::job::SyncJob;
use crate::model::preview::PreviewState;
use crate::model::session::SessionStatus;

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
    ui.add_space(6.0);

    if ui.button(t("＋ 新建任务", "＋ New Job")).clicked() {
        let name = if is_zh() {
            format!("任务 {}", app.config.jobs.len() + 1)
        } else {
            format!("Job {}", app.config.jobs.len() + 1)
        };
        let concurrency = app.config.settings.default_concurrency;
        app.config.jobs.push(SyncJob::new(name, concurrency));
        app.selected_job = Some(app.config.jobs.len() - 1);
        app.dirty = true;
    }

    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    let mut to_delete: Option<usize> = None;
    let mut to_duplicate: Option<usize> = None;
    let mut to_move_up: Option<usize> = None;
    let mut to_move_down: Option<usize> = None;
    let job_count = app.config.jobs.len();
    let selected = app.selected_job;

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            for (i, job) in app.config.jobs.iter().enumerate() {
                let is_selected = selected == Some(i);

                let label = egui::RichText::new(&job.name);
                let label = if is_selected { label.strong() } else { label };

                // 运行中：显示旋转指示器；完成/错误：显示小徽标
                let is_running = app.sync_running && selected == Some(i);
                let badge: Option<(&str, egui::Color32)> = if is_running {
                    Some((t("● 运行中", "● Running"), egui::Color32::GREEN))
                } else if selected == Some(i) {
                    if let Some(ref s) = app.session {
                        match s.status {
                            SessionStatus::Completed if s.stats.error_count == 0 => {
                                Some(("✓", egui::Color32::from_rgb(80, 200, 100)))
                            }
                            SessionStatus::Completed => Some(("⚠", egui::Color32::YELLOW)),
                            SessionStatus::Failed | SessionStatus::Stopped => {
                                Some(("✗", egui::Color32::RED))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                let resp = ui.selectable_label(is_selected, label);

                if let Some((badge_text, color)) = badge {
                    ui.label(
                        egui::RichText::new(format!("  {}", badge_text))
                            .small()
                            .color(color),
                    );
                }

                if resp.clicked() {
                    if app.selected_job != Some(i) {
                        app.preview_state = PreviewState::Idle;
                    }
                    app.selected_job = Some(i);
                    app.new_exclusion_input.clear();
                    app.exclusion_error = None;
                }

                // 右键菜单
                resp.context_menu(|ui| {
                    if i > 0 && ui.button(t("↑ 上移", "↑ Move Up")).clicked() {
                        to_move_up = Some(i);
                        ui.close_menu();
                    }
                    if i + 1 < job_count && ui.button(t("↓ 下移", "↓ Move Down")).clicked() {
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

                // 上次同步时间
                if let Some(t_time) = job.last_sync_time {
                    let formatted = t_time.format("%m-%d %H:%M").to_string();
                    let label_text = if is_zh() {
                        format!("  上次: {}", formatted)
                    } else {
                        format!("  Last: {}", formatted)
                    };
                    ui.label(
                        egui::RichText::new(label_text)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }

                // 定时同步：下次执行时间
                if job.schedule.enabled && job.schedule.interval_minutes > 0 {
                    let label_text = match job.last_sync_time {
                        Some(last) => {
                            let next = last
                                + chrono::Duration::minutes(
                                    job.schedule.interval_minutes as i64,
                                );
                            let next_local = next.with_timezone(&chrono::Local);
                            let now = chrono::Local::now();
                            if next_local <= now {
                                t("  ⏰ 即将同步...", "  ⏰ Syncing soon...").to_string()
                            } else {
                                let time = next_local.format("%H:%M");
                                if is_zh() {
                                    format!("  ⏰ 下次: {}", time)
                                } else {
                                    format!("  ⏰ Next: {}", time)
                                }
                            }
                        }
                        None => t("  ⏰ 定时（待首次运行）", "  ⏰ Scheduled (never run)")
                            .to_string(),
                    };
                    ui.label(
                        egui::RichText::new(label_text)
                            .small()
                            .color(egui::Color32::from_rgb(100, 180, 255)),
                    );
                }

                // 上次运行统计摘要
                if let Some(ref s) = job.last_run_summary {
                    let mut parts = if is_zh() {
                        vec![
                            format!("复制 {}", s.copied),
                            format!("跳过 {}", s.skipped),
                        ]
                    } else {
                        vec![
                            format!("Copied {}", s.copied),
                            format!("Skipped {}", s.skipped),
                        ]
                    };
                    if s.errors > 0 {
                        parts.push(if is_zh() {
                            format!("错误 {}", s.errors)
                        } else {
                            format!("Errors {}", s.errors)
                        });
                    }
                    if s.deleted > 0 {
                        parts.push(if is_zh() {
                            format!("删除 {}", s.deleted)
                        } else {
                            format!("Deleted {}", s.deleted)
                        });
                    }
                    let color = if s.errors > 0 {
                        egui::Color32::YELLOW
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    ui.label(
                        egui::RichText::new(format!("  {}", parts.join("  ")))
                            .small()
                            .color(color),
                    );
                }
            }
        });
    });

    // 上下移动
    if let Some(i) = to_move_up {
        app.config.jobs.swap(i, i - 1);
        if app.selected_job == Some(i) {
            app.selected_job = Some(i - 1);
        } else if app.selected_job == Some(i - 1) {
            app.selected_job = Some(i);
        }
        app.dirty = true;
    } else if let Some(i) = to_move_down {
        app.config.jobs.swap(i, i + 1);
        if app.selected_job == Some(i) {
            app.selected_job = Some(i + 1);
        } else if app.selected_job == Some(i + 1) {
            app.selected_job = Some(i);
        }
        app.dirty = true;
    }

    // 复制任务
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
        new_job.schedule.enabled = false;
        app.config.jobs.insert(i + 1, new_job);
        app.selected_job = Some(i + 1);
        app.dirty = true;
    }

    // 触发删除确认
    if let Some(idx) = to_delete {
        app.pending_delete = Some(idx);
    }

    // 删除确认弹窗
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
                    format!("确定要删除任务「{}」吗？此操作不可撤销。", job_name)
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
                        app.dirty = true;
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
