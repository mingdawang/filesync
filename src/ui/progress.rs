use egui::Ui;

use crate::app::{effective_copied_bytes, FileSyncApp};
use crate::i18n::{is_zh, t};
use crate::model::job::{RunTrigger, SyncMode};
use crate::model::session::{SessionStatus, WorkerState};

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
    let Some(job_idx) = app.selected_job else {
        ui.label(
            egui::RichText::new(t(
                "\u{5C31}\u{7EEA}\u{FF0C}\u{7B49}\u{5F85}\u{5F00}\u{59CB}\u{540C}\u{6B65}\u{3002}",
                "Ready, waiting to start sync.",
            ))
            .color(ui.visuals().weak_text_color()),
        );
        return;
    };
    let Some(job) = app.config.jobs.get(job_idx).cloned() else {
        return;
    };

    ui.horizontal(|ui| {
        ui.strong(t(
            "\u{540C}\u{6B65}\u{8FDB}\u{5EA6} / \u{65E5}\u{5FD7}",
            "Sync Progress / Log",
        ));

        if let Some(session) = &app.session {
            let (text, color) = session_status_badge(session.status.clone());
            ui.label(egui::RichText::new(text).color(color).small());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.sync_running
                && ui.small_button(t("\u{25A0} \u{505C}\u{6B62}", "\u{25A0} Stop")).clicked()
            {
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
        ui.label(
            egui::RichText::new(trigger_line(session.trigger, session.retry_attempt))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }

    if !app.job_queue.is_empty() {
        show_pending_queue(ui, app);
    }

    let Some(session) = &app.session else {
        return;
    };

    show_overall_progress(ui, session);
    show_worker_progress(ui, session);
    show_completion_summary(ui, session);
    show_error_log(ui, session);
}

fn show_pending_queue(ui: &mut Ui, app: &FileSyncApp) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t(
            "\u{5F85}\u{6267}\u{884C}\u{961F}\u{5217}",
            "Pending Queue",
        ))
        .small()
        .strong(),
    );
    for (pos, entry) in app.job_queue.iter().take(5).enumerate() {
        let name = app
            .config
            .jobs
            .iter()
            .find(|job| job.id == entry.job_id)
            .map(|job| job.name.as_str())
            .unwrap_or("?");
        let when = if entry.ready_at > chrono::Utc::now() {
            entry.ready_at
                .with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        } else {
            t("\u{7ACB}\u{5373}", "Now").to_string()
        };
        let text = format!(
            "{}. {}  [{}]  {}",
            pos + 1,
            name,
            trigger_label(entry.trigger),
            when
        );
        ui.label(egui::RichText::new(text).small().color(ui.visuals().weak_text_color()));
    }
}

fn show_overall_progress(ui: &mut Ui, session: &crate::model::session::SyncSession) {
    let stats = &session.stats;
    let progress = stats.progress();
    let elapsed_secs = session.started_at_instant.elapsed().as_secs_f64().max(1.0);
    let effective_bytes = effective_copied_bytes(session);
    let accounted_bytes = effective_bytes
        .saturating_add(stats.saved_bytes)
        .min(stats.total_bytes);

    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.label(t("\u{6587}\u{4EF6}:", "Files:"));
        let bar_width = (ui.available_width() * 0.90).max(600.0);
        let text = format!(
            "{} / {} ({:.1}%)",
            stats.processed_files,
            stats.total_files,
            progress * 100.0
        );
        ui.add(egui::ProgressBar::new(progress).desired_width(bar_width).text(text));
    });

    if stats.total_bytes > 0 {
        ui.horizontal(|ui| {
            ui.label(t("\u{6570}\u{636E}:", "Data:"));
            let bar_width = (ui.available_width() * 0.90).max(600.0);
            let bytes_progress = (accounted_bytes as f32 / stats.total_bytes as f32).min(1.0);
            ui.add(
                egui::ProgressBar::new(bytes_progress)
                    .desired_width(bar_width)
                    .text(format!(
                        "{} / {}",
                        fmt_bytes(effective_bytes),
                        fmt_bytes(stats.total_bytes)
                    )),
            );
        });
    }

    ui.horizontal_wrapped(|ui| {
        ui.label(stat_label(t("\u{590D}\u{5236}", "Copied"), stats.copied_files));
        ui.separator();
        ui.label(stat_label(t("\u{8DF3}\u{8FC7}", "Skipped"), stats.skipped_files));
        ui.separator();
        ui.label(stat_label(t("\u{5220}\u{9664}", "Deleted"), stats.deleted_files));
        ui.separator();
        ui.label(stat_label(
            t("\u{5DEE}\u{91CF}\u{8282}\u{7701}", "Delta saved"),
            fmt_bytes(stats.saved_bytes),
        ));
        ui.separator();
        ui.label(stat_label(
            t("\u{901F}\u{5EA6}", "Speed"),
            format!("{}/s", fmt_bytes(stats.speed_bps)),
        ));
        ui.separator();
        ui.label(
            egui::RichText::new(error_summary(stats))
                .color(if stats.error_count > 0 {
                    egui::Color32::from_rgb(255, 160, 50)
                } else {
                    ui.visuals().text_color()
                }),
        );
        if session.status == SessionStatus::Running
            && accounted_bytes < stats.total_bytes
            && stats.speed_bps > 0
        {
            let remaining = stats.total_bytes.saturating_sub(accounted_bytes);
            let eta_secs = (remaining as f64 / stats.speed_bps as f64).ceil() as u64;
            ui.separator();
            ui.label(stat_label("ETA", fmt_duration(eta_secs)));
        }
        if session.status == SessionStatus::Completed {
            ui.separator();
            ui.label(stat_label(
                t("\u{8017}\u{65F6}", "Elapsed"),
                fmt_duration(elapsed_secs as u64),
            ));
        }
    });
}

fn show_worker_progress(ui: &mut Ui, session: &crate::model::session::SyncSession) {
    let active: Vec<_> = session
        .active_workers
        .iter()
        .enumerate()
        .filter(|(_, worker)| !matches!(worker, WorkerState::Idle))
        .collect();
    if active.is_empty() {
        return;
    }

    ui.add_space(6.0);
    for (i, worker) in active {
        match worker {
            WorkerState::Copying {
                path,
                size,
                done,
                is_new,
            } => {
                let file_progress = if *size > 0 {
                    *done as f32 / *size as f32
                } else {
                    0.0
                };
                let suffix = fmt_bytes(*size);
                let label = format!(
                    "{} ({})",
                    truncate_filename_for_bar(
                        &display_name(path),
                        &suffix,
                        ui.available_width() * 0.90
                    ),
                    suffix
                );
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("[{}]", i + 1)).small().monospace());
                    ui.label(
                        egui::RichText::new(if *is_new {
                            t("\u{65B0}\u{589E}", "New")
                        } else {
                            t("\u{8986}\u{76D6}", "Update")
                        })
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
                let suffix = if *is_dir {
                    t("\u{76EE}\u{5F55}", "Dir")
                } else {
                    t("\u{6587}\u{4EF6}", "File")
                };
                let label = format!(
                    "{} ({})",
                    truncate_filename_for_bar(
                        &display_name(path),
                        suffix,
                        ui.available_width() * 0.90
                    ),
                    suffix
                );
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("[{}]", i + 1)).small().monospace());
                    ui.label(
                        egui::RichText::new(t("\u{5220}\u{9664}", "Delete"))
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

fn show_completion_summary(ui: &mut Ui, session: &crate::model::session::SyncSession) {
    if session.status != SessionStatus::Completed {
        return;
    }

    let stats = &session.stats;
    let elapsed_secs = session.started_at_instant.elapsed().as_secs() as u64;
    ui.add_space(6.0);
    let text = if is_zh() {
        format!(
            "\u{540C}\u{6B65}\u{5B8C}\u{6210}\u{FF1A}\u{590D}\u{5236} {}\u{FF0C}\u{8DF3}\u{8FC7} {}\u{FF0C}\u{5220}\u{9664} {}\u{FF0C}\u{8017}\u{65F6} {}",
            stats.copied_files,
            stats.skipped_files,
            stats.deleted_files,
            fmt_duration(elapsed_secs)
        )
    } else {
        format!(
            "Sync complete: copied {}, skipped {}, deleted {}, elapsed {}",
            stats.copied_files,
            stats.skipped_files,
            stats.deleted_files,
            fmt_duration(elapsed_secs)
        )
    };
    ui.label(egui::RichText::new(text).color(egui::Color32::from_rgb(100, 200, 100)));
}

fn show_error_log(ui: &mut Ui, session: &crate::model::session::SyncSession) {
    if session.errors.is_empty() {
        return;
    }

    ui.add_space(6.0);
    ui.separator();
    ui.label(
        egui::RichText::new(if is_zh() {
            format!(
                "\u{9519}\u{8BEF}\u{65E5}\u{5FD7} ({})",
                session.errors.len()
            )
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

fn strategy_summary(job: &crate::model::job::SyncJob) -> String {
    let mode = match job.sync_mode {
        SyncMode::Update => t("\u{589E}\u{91CF}\u{540C}\u{6B65}", "Update"),
        SyncMode::Mirror => t("\u{955C}\u{50CF}\u{540C}\u{6B65}", "Mirror"),
    };
    let compare = match job.compare_method {
        crate::model::config::CompareMethod::Metadata => {
            t("\u{5143}\u{6570}\u{636E}\u{6BD4}\u{8F83}", "Metadata compare")
        }
        crate::model::config::CompareMethod::Hash => {
            t("\u{5185}\u{5BB9}\u{54C8}\u{5E0C}\u{6BD4}\u{8F83}", "Content-hash compare")
        }
    };
    let verify = if job.engine_options.verify_after_copy {
        t(
            "\u{590D}\u{5236}\u{540E}\u{6821}\u{9A8C}",
            "verify after copy",
        )
    } else {
        t(
            "\u{4E0D}\u{505A}\u{590D}\u{5236}\u{540E}\u{6821}\u{9A8C}",
            "no post-copy verification",
        )
    };
    if is_zh() {
        format!("\u{7B56}\u{7565}: {} / {} / {}", mode, compare, verify)
    } else {
        format!("Strategy: {} / {} / {}", mode, compare, verify)
    }
}

fn session_status_badge(status: SessionStatus) -> (&'static str, egui::Color32) {
    match status {
        SessionStatus::Running => (
            t("\u{25CF} \u{8FD0}\u{884C}\u{4E2D}", "\u{25CF} Running"),
            egui::Color32::GREEN,
        ),
        SessionStatus::Paused => (
            t("\u{23F8} \u{5DF2}\u{6682}\u{505C}", "\u{23F8} Paused"),
            egui::Color32::YELLOW,
        ),
        SessionStatus::Completed => (
            t("\u{2713} \u{5DF2}\u{5B8C}\u{6210}", "\u{2713} Completed"),
            egui::Color32::from_rgb(100, 200, 100),
        ),
        SessionStatus::Failed => (
            t("\u{2715} \u{5931}\u{8D25}", "\u{2715} Failed"),
            egui::Color32::RED,
        ),
        SessionStatus::Stopped => (
            t("\u{25A0} \u{5DF2}\u{505C}\u{6B62}", "\u{25A0} Stopped"),
            egui::Color32::GRAY,
        ),
    }
}

fn trigger_line(trigger: RunTrigger, retry_attempt: u32) -> String {
    let trigger_text = match trigger {
        RunTrigger::Manual => t("\u{624B}\u{52A8}\u{89E6}\u{53D1}", "Manual trigger"),
        RunTrigger::Scheduled => t("\u{5B9A}\u{65F6}\u{89E6}\u{53D1}", "Scheduled trigger"),
        RunTrigger::Retry => t("\u{5931}\u{8D25}\u{91CD}\u{8BD5}", "Retry trigger"),
    };
    if retry_attempt > 0 {
        if is_zh() {
            format!(
                "\u{5F53}\u{524D}\u{6765}\u{6E90}: {}  \u{7B2C} {} \u{6B21}\u{91CD}\u{8BD5}",
                trigger_text, retry_attempt
            )
        } else {
            format!("Current source: {}  Retry {}", trigger_text, retry_attempt)
        }
    } else if is_zh() {
        format!("\u{5F53}\u{524D}\u{6765}\u{6E90}: {}", trigger_text)
    } else {
        format!("Current source: {}", trigger_text)
    }
}

fn trigger_label(trigger: RunTrigger) -> &'static str {
    match trigger {
        RunTrigger::Manual => t("\u{624B}\u{52A8}", "Manual"),
        RunTrigger::Scheduled => t("\u{5B9A}\u{65F6}", "Scheduled"),
        RunTrigger::Retry => t("\u{91CD}\u{8BD5}", "Retry"),
    }
}

fn error_summary(stats: &crate::model::session::SyncStats) -> String {
    if is_zh() {
        format!(
            "\u{9519}\u{8BEF} {} (\u{626B}\u{63CF} {} / \u{590D}\u{5236} {} / \u{5220}\u{9664} {})",
            stats.error_count,
            stats.scan_error_count,
            stats.copy_error_count,
            stats.delete_error_count
        )
    } else {
        format!(
            "Errors {} (scan {} / copy {} / delete {})",
            stats.error_count,
            stats.scan_error_count,
            stats.copy_error_count,
            stats.delete_error_count
        )
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
    let max_chars = ((bar_width / 7.5) as usize)
        .saturating_sub(reserved_chars)
        .max(12);
    if filename.chars().count() <= max_chars {
        return filename.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let truncated: String = filename.chars().take(keep).collect();
    format!("{}...", truncated)
}
