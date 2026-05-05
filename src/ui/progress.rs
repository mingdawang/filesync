use egui::Ui;

use crate::app::{effective_copied_bytes, FileSyncApp};
use crate::i18n::{is_zh, t};
use crate::model::job::SyncMode;
use crate::model::session::{SessionStatus, WorkerState};

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
    let is_mirror = app
        .selected_job
        .and_then(|idx| app.config.jobs.get(idx))
        .map(|j| j.sync_mode == SyncMode::Mirror)
        .unwrap_or(false);
    // ── 标题行 + 控制按钮 ─────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.strong(t("同步进度 / 日志", "Sync Progress / Log"));

        if let Some(session) = &app.session {
            let (text, color) = match session.status {
                SessionStatus::Running => (t("● 运行中", "● Running"), egui::Color32::GREEN),
                SessionStatus::Paused => (t("⏸ 已暂停", "⏸ Paused"), egui::Color32::YELLOW),
                SessionStatus::Completed => {
                    (t("✓ 已完成", "✓ Completed"), egui::Color32::from_rgb(100, 200, 100))
                }
                SessionStatus::Failed => (t("✗ 失败", "✗ Failed"), egui::Color32::RED),
                SessionStatus::Stopped => (t("■ 已停止", "■ Stopped"), egui::Color32::GRAY),
            };
            ui.label(egui::RichText::new(text).color(color).small());
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if app.sync_running {
                if ui.small_button(t("■ 停止", "■ Stop")).clicked() {
                    app.stop_sync();
                }
            }
        });
    });

    // ── 就绪状态 ──────────────────────────────────────────────────
    let Some(session) = &app.session else {
        ui.label(
            egui::RichText::new(t("就绪，等待开始同步。", "Ready, waiting to start sync."))
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };

    let stats = &session.stats;
    let progress = stats.progress();
    let elapsed_secs = session.started_at_instant.elapsed().as_secs_f64().max(1.0);
    let effective_bytes = effective_copied_bytes(session);
    let accounted_bytes =
        (effective_bytes.saturating_add(stats.saved_bytes)).min(stats.total_bytes);

    ui.add_space(4.0);

    // ── 文件数进度条 ───────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(t("文件:", "Files:"));
        let bar_width = ui.available_width().max(450.0);
        let progress_text = if is_zh() {
            format!(
                "{} / {} 个  ({:.1}%)",
                stats.processed_files,
                stats.total_files,
                progress * 100.0
            )
        } else {
            format!(
                "{} / {}  ({:.1}%)",
                stats.processed_files,
                stats.total_files,
                progress * 100.0
            )
        };
        ui.add(
            egui::ProgressBar::new(progress)
                .desired_width(bar_width)
                .text(progress_text),
        );
    });

    // ── 字节进度条 ────────────────────────────────────────────────
    if stats.total_bytes > 0 {
        let bytes_progress = (accounted_bytes as f32 / stats.total_bytes as f32).min(1.0);
        ui.horizontal(|ui| {
            ui.label(t("数据:", "Data:"));
            let bar_width = ui.available_width().max(450.0);
            ui.add(
                egui::ProgressBar::new(bytes_progress)
                    .desired_width(bar_width)
                    .text(format!(
                        "{} / {}",
                        fmt_bytes(effective_bytes),
                        fmt_bytes(stats.total_bytes),
                    )),
            );
        });
    }

    // ── 统计行 ────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(if is_zh() {
            format!("复制: {}", stats.copied_files)
        } else {
            format!("Copied: {}", stats.copied_files)
        });
        ui.separator();
        ui.label(if is_zh() {
            format!("跳过: {}", stats.skipped_files)
        } else {
            format!("Skipped: {}", stats.skipped_files)
        });
        ui.separator();
        ui.label(
            egui::RichText::new(if is_zh() {
                format!("错误: {}", stats.error_count)
            } else {
                format!("Errors: {}", stats.error_count)
            })
            .color(if stats.error_count > 0 {
                egui::Color32::from_rgb(255, 160, 50)
            } else {
                ui.visuals().text_color()
            }),
        );
        if stats.total_bytes > 0 {
            ui.separator();
            ui.label(if is_zh() {
                format!("总量: {}", fmt_bytes(stats.total_bytes))
            } else {
                format!("Total: {}", fmt_bytes(stats.total_bytes))
            });
        }
        ui.separator();
        ui.label(
            egui::RichText::new(if is_zh() {
                format!("差量节省: {}", fmt_bytes(stats.saved_bytes))
            } else {
                format!("Delta saved: {}", fmt_bytes(stats.saved_bytes))
            })
            .color(egui::Color32::from_rgb(100, 220, 100)),
        );
        if stats.deleted_files > 0 {
            ui.separator();
            ui.label(
                egui::RichText::new(if is_zh() {
                    format!("已删除: {}", stats.deleted_files)
                } else {
                    format!("Deleted: {}", stats.deleted_files)
                })
                .color(egui::Color32::from_rgb(255, 140, 60)),
            );
        }
        if !is_mirror && stats.orphan_files > 0 {
            ui.separator();
            ui.label(
                egui::RichText::new(if is_zh() {
                    format!("孤立: {}", stats.orphan_files)
                } else {
                    format!("Orphans: {}", stats.orphan_files)
                })
                .color(ui.visuals().weak_text_color()),
            );
        }
        if stats.speed_bps > 0 {
            ui.separator();
            ui.label(format!("{}/s", fmt_bytes(stats.speed_bps)));
        }

        // ETA
        if session.status == SessionStatus::Running
            && accounted_bytes < stats.total_bytes
        {
            let current_speed = stats.speed_bps.max((effective_bytes as f64 / elapsed_secs) as u64);
            if current_speed > 0 {
                let remaining_bytes =
                    stats.total_bytes.saturating_sub(accounted_bytes);
                let eta_secs =
                    (remaining_bytes as f64 / current_speed as f64).ceil() as u64;
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("ETA {}", fmt_duration(eta_secs)))
                        .color(ui.visuals().weak_text_color()),
                );
            }
        }
    });

    // ── 活跃 Worker ───────────────────────────────────────────────
    let active: Vec<_> = session
        .active_workers
        .iter()
        .enumerate()
        .filter(|(_, w)| matches!(w, WorkerState::Copying { .. }))
        .collect();

    if !active.is_empty() {
        ui.add_space(4.0);
        for (i, worker) in &active {
            if let WorkerState::Copying { path, size, done, is_new } = worker {
                let file_progress =
                    if *size > 0 { *done as f32 / *size as f32 } else { 0.0 };
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                let action_label = if *is_new {
                    t("新增", "New")
                } else {
                    t("覆盖", "Upd")
                };

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("[{}]", i + 1)).small().monospace(),
                    );
                    ui.label(
                        egui::RichText::new(action_label)
                            .small()
                            .color(if *is_new {
                                egui::Color32::from_rgb(80, 200, 120)
                            } else {
                                egui::Color32::from_rgb(100, 180, 255)
                            }),
                    );
                    let bar_width = ui.available_width().max(450.0);
                    let size_text = fmt_bytes(*size);
                    let label_text = format!(
                        "{} ({})",
                        truncate_filename_for_bar(&filename, &size_text, bar_width),
                        size_text
                    );
                    ui.add(
                        egui::ProgressBar::new(file_progress)
                            .desired_width(bar_width)
                            .text(label_text),
                    );
                });
            }
        }
    }

    // ── 完成摘要 ──────────────────────────────────────────────────
    if session.status == SessionStatus::Completed {
        ui.add_space(4.0);
        let elapsed = fmt_duration(elapsed_secs as u64);
        let orphan_suffix = if !is_mirror && stats.orphan_files > 0 {
            if is_zh() {
                format!("，孤立 {} 个（未删除）", stats.orphan_files)
            } else {
                format!(", {} orphan(s) kept", stats.orphan_files)
            }
        } else {
            String::new()
        };
        let delete_suffix = if is_mirror && stats.deleted_files > 0 {
            if is_zh() {
                format!("，删除孤立 {} 个", stats.deleted_files)
            } else {
                format!(", deleted {} orphan(s)", stats.deleted_files)
            }
        } else {
            String::new()
        };
        let summary = if is_zh() {
            if stats.error_count == 0 {
                format!(
                    "同步完成：复制 {} 个文件，跳过 {} 个，耗时 {}{}{}",
                    stats.copied_files, stats.skipped_files, elapsed, orphan_suffix, delete_suffix
                )
            } else {
                format!(
                    "同步完成：复制 {} 个，跳过 {} 个，错误 {} 个，耗时 {}{}{}",
                    stats.copied_files, stats.skipped_files, stats.error_count, elapsed, orphan_suffix, delete_suffix
                )
            }
        } else if stats.error_count == 0 {
            format!(
                "Sync complete: copied {}, skipped {}, elapsed {}{}{}",
                stats.copied_files, stats.skipped_files, elapsed, orphan_suffix, delete_suffix
            )
        } else {
            format!(
                "Sync complete: copied {}, skipped {}, errors {}, elapsed {}{}{}",
                stats.copied_files, stats.skipped_files, stats.error_count, elapsed, orphan_suffix, delete_suffix
            )
        };
        ui.label(egui::RichText::new(summary).color(egui::Color32::from_rgb(100, 200, 100)));
    }

    // ── 已删除文件日志（Mirror 模式）────────────────────────────────
    if is_mirror && !session.deleted_paths.is_empty() {
        ui.add_space(4.0);
        ui.separator();
        ui.label(
            egui::RichText::new(if is_zh() {
                format!("已删除文件 ({} 个)", session.deleted_paths.len())
            } else {
                format!("Deleted files ({} total)", session.deleted_paths.len())
            })
            .small()
            .color(egui::Color32::from_rgb(255, 140, 60)),
        );
        let show_count = session.deleted_paths.len().min(DELETED_LOG_LIMIT);
        let start = session.deleted_paths.len().saturating_sub(show_count);
        egui::ScrollArea::vertical()
            .id_salt("deleted_log")
            .max_height(120.0)
            .show(ui, |ui| {
                for path in &session.deleted_paths[start..] {
                    ui.label(
                        egui::RichText::new(format!("✕ {}", path.display()))
                            .small()
                            .monospace()
                            .color(egui::Color32::from_rgb(200, 120, 60)),
                    );
                }
            });
    }

    // ── 错误日志 ──────────────────────────────────────────────────
    let errors = &session.errors;
    if !errors.is_empty() {
        ui.add_space(4.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(if is_zh() {
                    format!("错误日志 ({} 条)", errors.len())
                } else {
                    format!("Error log ({} entries)", errors.len())
                })
                .small()
                .color(egui::Color32::from_rgb(255, 160, 50)),
            );
            if ui.small_button(t("复制", "Copy")).clicked() {
                let log = build_log_text(errors);
                ui.output_mut(|o| o.copied_text = log);
            }
            if ui.small_button(t("保存", "Save")).clicked() {
                let log = build_log_text(errors);
                save_log_to_file(&log);
            }
        });

        let show_count = errors.len().min(ERROR_LOG_LIMIT);
        let start = errors.len().saturating_sub(show_count);
        egui::ScrollArea::vertical()
            .id_salt("error_log")
            .max_height(100.0)
            .show(ui, |ui| {
                for err in &errors[start..] {
                    ui.label(
                        egui::RichText::new(format!(
                            "⚠ [{}] {} — {}",
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
}

// ─────────────────────────────────────────────────────────────────
// 辅助函数
// ─────────────────────────────────────────────────────────────────

/// 已删除文件日志最多显示条数
const DELETED_LOG_LIMIT: usize = 200;
/// 错误日志最多显示条数
const ERROR_LOG_LIMIT: usize = 100;

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

fn truncate_filename_for_bar(filename: &str, size_text: &str, bar_width: f32) -> String {
    let reserved_chars = size_text.chars().count() + 6;
    let max_chars = ((bar_width / 7.5) as usize).saturating_sub(reserved_chars).max(12);
    let char_count = filename.chars().count();
    if char_count <= max_chars {
        return filename.to_string();
    }

    let keep = max_chars.saturating_sub(1);
    let truncated: String = filename.chars().take(keep).collect();
    format!("{}…", truncated)
}

fn build_log_text(errors: &[crate::model::session::SyncError]) -> String {
    errors
        .iter()
        .map(|e| {
            format!(
                "[{}] {} — {}",
                e.timestamp.format("%H:%M:%S"),
                e.path.display(),
                e.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn save_log_to_file(log: &str) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title(t("保存错误日志", "Save Error Log"))
        .add_filter(t("文本文件", "Text files"), &["txt"])
        .add_filter(t("所有文件", "All files"), &["*"])
        .set_file_name("filesync_errors.txt")
        .save_file()
    {
        let _ = std::fs::write(path, log);
    }
}
