use egui::Ui;

use crate::app::{effective_copied_bytes, FileSyncApp};
use crate::i18n::{is_zh, t};
use crate::model::job::{RunResultStatus, RunTrigger, SyncMode};
use crate::model::session::{SessionStatus, WorkerState};

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
    let Some(job_idx) = app.selected_job else {
        ui.label(
            egui::RichText::new(t("就绪，等待开始同步。", "Ready, waiting to start sync."))
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };
    let Some(job) = app.config.jobs.get(job_idx).cloned() else {
        return;
    };

    ui.horizontal(|ui| {
        ui.strong(t("同步进度 / 日志", "Sync Progress / Log"));

        if let Some(session) = &app.session {
            let (text, color) = match session.status {
                SessionStatus::Running => (t("● 运行中", "● Running"), egui::Color32::GREEN),
                SessionStatus::Paused => (t("⏸ 已暂停", "⏸ Paused"), egui::Color32::YELLOW),
                SessionStatus::Completed => (t("✓ 已完成", "✓ Completed"), egui::Color32::from_rgb(100, 200, 100)),
                SessionStatus::Failed => (t("✕ 失败", "✕ Failed"), egui::Color32::RED),
                SessionStatus::Stopped => (t("■ 已停止", "■ Stopped"), egui::Color32::GRAY),
            };
            ui.label(egui::RichText::new(text).color(color).small());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.sync_running && ui.small_button(t("■ 停止", "■ Stop")).clicked() {
                app.stop_sync();
            }
        });
    });

    ui.label(
        egui::RichText::new(strategy_summary(&job))
            .small()
            .color(ui.visuals().weak_text_color()),
    );

    if let Some(session) = &app.session {
        let trigger_text = match session.trigger {
            RunTrigger::Manual => t("手动触发", "Manual trigger"),
            RunTrigger::Scheduled => t("定时触发", "Scheduled trigger"),
            RunTrigger::Retry => t("失败重试", "Retry trigger"),
        };
        let trigger_line = if session.retry_attempt > 0 {
            if is_zh() {
                format!("当前来源: {}  第 {} 次重试", trigger_text, session.retry_attempt)
            } else {
                format!("Current source: {}  Retry {}", trigger_text, session.retry_attempt)
            }
        } else if is_zh() {
            format!("当前来源: {}", trigger_text)
        } else {
            format!("Current source: {}", trigger_text)
        };
        ui.label(egui::RichText::new(trigger_line).small().color(ui.visuals().weak_text_color()));
    }

    if !app.job_queue.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(t("待执行队列", "Pending Queue")).small().strong());
        for (pos, entry) in app.job_queue.iter().take(5).enumerate() {
            let name = app
                .config
                .jobs
                .get(entry.job_idx)
                .map(|job| job.name.as_str())
                .unwrap_or("?");
            let trigger = match entry.trigger {
                RunTrigger::Manual => t("手动", "Manual"),
                RunTrigger::Scheduled => t("定时", "Scheduled"),
                RunTrigger::Retry => t("重试", "Retry"),
            };
            let when = if entry.ready_at > chrono::Utc::now() {
                entry.ready_at.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string()
            } else {
                t("立即", "Now").to_string()
            };
            let text = if is_zh() {
                format!("{}. {}  [{}]  {}", pos + 1, name, trigger, when)
            } else {
                format!("{}. {}  [{}]  {}", pos + 1, name, trigger, when)
            };
            ui.label(egui::RichText::new(text).small().color(ui.visuals().weak_text_color()));
        }
    }

    let Some(session) = &app.session else {
        show_history(ui, &job);
        return;
    };

    let stats = &session.stats;
    let progress = stats.progress();
    let elapsed_secs = session.started_at_instant.elapsed().as_secs_f64().max(1.0);
    let effective_bytes = effective_copied_bytes(session);
    let accounted_bytes = effective_bytes.saturating_add(stats.saved_bytes).min(stats.total_bytes);

    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.label(t("文件:", "Files:"));
        let bar_width = (ui.available_width() * 0.90).max(600.0);
        let text = if is_zh() {
            format!("{} / {} ({:.1}%)", stats.processed_files, stats.total_files, progress * 100.0)
        } else {
            format!("{} / {} ({:.1}%)", stats.processed_files, stats.total_files, progress * 100.0)
        };
        ui.add(egui::ProgressBar::new(progress).desired_width(bar_width).text(text));
    });

    if stats.total_bytes > 0 {
        ui.horizontal(|ui| {
            ui.label(t("数据:", "Data:"));
            let bar_width = (ui.available_width() * 0.90).max(600.0);
            let bytes_progress = (accounted_bytes as f32 / stats.total_bytes as f32).min(1.0);
            ui.add(
                egui::ProgressBar::new(bytes_progress)
                    .desired_width(bar_width)
                    .text(format!("{} / {}", fmt_bytes(effective_bytes), fmt_bytes(stats.total_bytes))),
            );
        });
    }

    ui.horizontal_wrapped(|ui| {
        ui.label(stat_label(t("复制", "Copied"), stats.copied_files));
        ui.separator();
        ui.label(stat_label(t("跳过", "Skipped"), stats.skipped_files));
        ui.separator();
        ui.label(stat_label(t("删除", "Deleted"), stats.deleted_files));
        ui.separator();
        ui.label(stat_label(t("差量节省", "Delta saved"), fmt_bytes(stats.saved_bytes)));
        ui.separator();
        ui.label(stat_label(t("速度", "Speed"), format!("{}/s", fmt_bytes(stats.speed_bps))));
        ui.separator();
        ui.label(
            egui::RichText::new(if is_zh() {
                format!(
                    "错误 {} (扫描 {} / 复制 {} / 删除 {})",
                    stats.error_count, stats.scan_error_count, stats.copy_error_count, stats.delete_error_count
                )
            } else {
                format!(
                    "Errors {} (scan {} / copy {} / delete {})",
                    stats.error_count, stats.scan_error_count, stats.copy_error_count, stats.delete_error_count
                )
            })
            .color(if stats.error_count > 0 {
                egui::Color32::from_rgb(255, 160, 50)
            } else {
                ui.visuals().text_color()
            }),
        );
        if session.status == SessionStatus::Running && accounted_bytes < stats.total_bytes && stats.speed_bps > 0 {
            let remaining = stats.total_bytes.saturating_sub(accounted_bytes);
            let eta_secs = (remaining as f64 / stats.speed_bps as f64).ceil() as u64;
            ui.separator();
            ui.label(stat_label("ETA", fmt_duration(eta_secs)));
        }
    });

    let active: Vec<_> = session
        .active_workers
        .iter()
        .enumerate()
        .filter(|(_, worker)| !matches!(worker, WorkerState::Idle))
        .collect();
    if !active.is_empty() {
        ui.add_space(6.0);
        for (i, worker) in active {
            match worker {
                WorkerState::Copying { path, size, done, is_new } => {
                    let file_progress = if *size > 0 { *done as f32 / *size as f32 } else { 0.0 };
                    let size_text = fmt_bytes(*size);
                    let label = format!(
                        "{} ({})",
                        truncate_filename_for_bar(&display_name(path), &size_text, ui.available_width() * 0.90),
                        size_text
                    );
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("[{}]", i + 1)).small().monospace());
                        ui.label(
                            egui::RichText::new(if *is_new { t("新增", "New") } else { t("覆盖", "Update") })
                                .small()
                                .color(if *is_new {
                                    egui::Color32::from_rgb(80, 200, 120)
                                } else {
                                    egui::Color32::from_rgb(100, 180, 255)
                                }),
                        );
                        ui.add(
                            egui::ProgressBar::new(file_progress)
                                .desired_width((ui.available_width() * 0.90).max(600.0))
                                .text(label),
                        );
                    });
                }
                WorkerState::Deleting { path, is_dir } => {
                    let suffix = if *is_dir { t("目录", "Dir") } else { t("文件", "File") };
                    let label = format!(
                        "{} ({})",
                        truncate_filename_for_bar(&display_name(path), suffix, ui.available_width() * 0.90),
                        suffix
                    );
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("[{}]", i + 1)).small().monospace());
                        ui.label(
                            egui::RichText::new(t("删除", "Delete"))
                                .small()
                                .color(egui::Color32::from_rgb(255, 140, 60)),
                        );
                        ui.add(
                            egui::ProgressBar::new(1.0)
                                .desired_width((ui.available_width() * 0.90).max(600.0))
                                .text(label),
                        );
                    });
                }
                WorkerState::Idle => {}
            }
        }
    }

    if session.status == SessionStatus::Completed {
        ui.add_space(6.0);
        let text = if is_zh() {
            format!(
                "同步完成：复制 {}，跳过 {}，删除 {}，耗时 {}",
                stats.copied_files,
                stats.skipped_files,
                stats.deleted_files,
                fmt_duration(elapsed_secs as u64)
            )
        } else {
            format!(
                "Sync complete: copied {}, skipped {}, deleted {}, elapsed {}",
                stats.copied_files,
                stats.skipped_files,
                stats.deleted_files,
                fmt_duration(elapsed_secs as u64)
            )
        };
        ui.label(egui::RichText::new(text).color(egui::Color32::from_rgb(100, 200, 100)));
    }

    if !session.errors.is_empty() {
        ui.add_space(6.0);
        ui.separator();
        ui.label(
            egui::RichText::new(if is_zh() {
                format!("错误日志 ({})", session.errors.len())
            } else {
                format!("Error log ({})", session.errors.len())
            })
            .small()
            .color(egui::Color32::from_rgb(255, 160, 50)),
        );
        egui::ScrollArea::vertical()
            .id_salt("error_log")
            .max_height(120.0)
            .show(ui, |ui| {
                for err in session.errors.iter().rev().take(100).rev() {
                    ui.label(
                        egui::RichText::new(format!(
                            "[{}] {} - {}",
                            err.timestamp.format("%H:%M:%S"),
                            err.path.display(),
                            err.message
                        ))
                        .small()
                        .color(egui::Color32::from_rgb(255, 160, 50)),
                    );
                }
            });
    }

    show_history(ui, &job);
}

fn show_history(ui: &mut Ui, job: &crate::model::job::SyncJob) {
    if job.run_history.is_empty() {
        return;
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new(t("最近运行", "Recent Runs")).small().strong());
    for entry in job.run_history.iter().take(5) {
        let result = match entry.result {
            RunResultStatus::Completed => t("成功", "Success"),
            RunResultStatus::Warning => t("告警", "Warning"),
            RunResultStatus::Failed => t("失败", "Failed"),
            RunResultStatus::Stopped => t("停止", "Stopped"),
            RunResultStatus::Missed => t("漏跑", "Missed"),
        };
        let trigger = match entry.trigger {
            RunTrigger::Manual => t("手动", "Manual"),
            RunTrigger::Scheduled => t("定时", "Scheduled"),
            RunTrigger::Retry => t("重试", "Retry"),
        };
        let text = if is_zh() {
            format!(
                "{}  [{} / {}]  {}",
                entry.finished_at.with_timezone(&chrono::Local).format("%m-%d %H:%M"),
                trigger,
                result,
                entry.note
            )
        } else {
            format!(
                "{}  [{} / {}]  {}",
                entry.finished_at.with_timezone(&chrono::Local).format("%m-%d %H:%M"),
                trigger,
                result,
                entry.note
            )
        };
        ui.label(egui::RichText::new(text).small().color(ui.visuals().weak_text_color()));
    }
}

fn strategy_summary(job: &crate::model::job::SyncJob) -> String {
    let mode = match job.sync_mode {
        SyncMode::Update => t("增量同步", "Update"),
        SyncMode::Mirror => t("镜像同步", "Mirror"),
    };
    let compare = match job.compare_method {
        crate::model::config::CompareMethod::Metadata => t("元数据比较", "Metadata compare"),
        crate::model::config::CompareMethod::Hash => t("内容哈希比较", "Content-hash compare"),
    };
    let verify = if job.engine_options.verify_after_copy {
        t("复制后校验", "verify after copy")
    } else {
        t("不做复制后校验", "no post-copy verification")
    };
    if is_zh() {
        format!("策略: {} / {} / {}", mode, compare, verify)
    } else {
        format!("Strategy: {} / {} / {}", mode, compare, verify)
    }
}

fn stat_label<T: std::fmt::Display>(name: &str, value: T) -> String {
    format!("{}: {}", name, value)
}

fn fmt_bytes(bytes: u64) -> String {
    super::fmt_bytes(bytes)
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn truncate_filename_for_bar(filename: &str, suffix_text: &str, bar_width: f32) -> String {
    let reserved_chars = suffix_text.chars().count() + 6;
    let max_chars = ((bar_width / 7.5) as usize).saturating_sub(reserved_chars).max(12);
    if filename.chars().count() <= max_chars {
        return filename.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let truncated: String = filename.chars().take(keep).collect();
    format!("{}...", truncated)
}
