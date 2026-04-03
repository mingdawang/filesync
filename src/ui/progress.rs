use chrono::Utc;
use egui::Ui;

use crate::app::FileSyncApp;
use crate::i18n::{is_zh, t};
use crate::model::session::{SessionStatus, WorkerState};

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
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
    let elapsed_secs = (Utc::now() - session.started_at).num_seconds().max(1) as f64;

    ui.add_space(4.0);

    // ── 文件数进度条 ───────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(t("文件:", "Files:"));
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
                .desired_width(340.0)
                .text(progress_text),
        );
    });

    // ── 字节进度条 ────────────────────────────────────────────────
    if stats.total_bytes > 0 {
        let bytes_progress =
            (stats.copied_bytes as f32 / stats.total_bytes as f32).min(1.0);
        ui.horizontal(|ui| {
            ui.label(t("数据:", "Data:"));
            ui.add(
                egui::ProgressBar::new(bytes_progress)
                    .desired_width(340.0)
                    .text(format!(
                        "{} / {}",
                        fmt_bytes(stats.copied_bytes),
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
                egui::Color32::YELLOW
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
        if stats.saved_bytes > 0 {
            ui.separator();
            ui.label(
                egui::RichText::new(if is_zh() {
                    format!("差量节省: {}", fmt_bytes(stats.saved_bytes))
                } else {
                    format!("Delta saved: {}", fmt_bytes(stats.saved_bytes))
                })
                .color(egui::Color32::from_rgb(100, 220, 100)),
            );
        }
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
        if stats.speed_bps > 0 {
            ui.separator();
            ui.label(format!("{}/s", fmt_bytes(stats.speed_bps)));
        }

        // ETA
        if session.status == SessionStatus::Running
            && stats.processed_files > 0
            && stats.total_files > stats.processed_files
        {
            let rate = stats.processed_files as f64 / elapsed_secs;
            let remaining = (stats.total_files - stats.processed_files) as f64;
            let eta_secs = (remaining / rate).ceil() as u64;
            ui.separator();
            ui.label(
                egui::RichText::new(format!("ETA {}", fmt_duration(eta_secs)))
                    .color(ui.visuals().weak_text_color()),
            );
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
            if let WorkerState::Copying { path, size, done } = worker {
                let file_progress =
                    if *size > 0 { *done as f32 / *size as f32 } else { 0.0 };
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("[{}]", i + 1)).small().monospace(),
                    );
                    ui.add(
                        egui::ProgressBar::new(file_progress)
                            .desired_width(240.0)
                            .text(format!("{} ({})", filename, fmt_bytes(*size))),
                    );
                });
            }
        }
    }

    // ── 完成摘要 ──────────────────────────────────────────────────
    if session.status == SessionStatus::Completed {
        ui.add_space(4.0);
        let elapsed = fmt_duration(elapsed_secs as u64);
        let summary = if is_zh() {
            if stats.error_count == 0 {
                format!(
                    "同步完成：复制 {} 个文件，跳过 {} 个，耗时 {}",
                    stats.copied_files, stats.skipped_files, elapsed
                )
            } else {
                format!(
                    "同步完成：复制 {} 个，跳过 {} 个，错误 {} 个，耗时 {}",
                    stats.copied_files, stats.skipped_files, stats.error_count, elapsed
                )
            }
        } else if stats.error_count == 0 {
            format!(
                "Sync complete: copied {}, skipped {}, elapsed {}",
                stats.copied_files, stats.skipped_files, elapsed
            )
        } else {
            format!(
                "Sync complete: copied {}, skipped {}, errors {}, elapsed {}",
                stats.copied_files, stats.skipped_files, stats.error_count, elapsed
            )
        };
        ui.label(egui::RichText::new(summary).color(egui::Color32::from_rgb(100, 200, 100)));
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
                .color(egui::Color32::YELLOW),
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

        let show_count = errors.len().min(100);
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
                        .color(egui::Color32::YELLOW),
                    );
                }
            });
    }
}

// ─────────────────────────────────────────────────────────────────
// 辅助函数
// ─────────────────────────────────────────────────────────────────

fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
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
