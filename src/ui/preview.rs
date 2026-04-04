/// 同步预览窗口：显示即将执行的操作列表
///
/// 当用户点击"预览"按钮时，后台扫描差异并填入 `PreviewState`，
/// 此模块负责渲染该窗口。

use chrono::{DateTime, Local};

use crate::app::FileSyncApp;
use crate::engine::diff::DiffAction;
use crate::i18n::{is_zh, t};
use crate::model::preview::PreviewState;

/// 渲染预览浮动窗口（在 app.rs 的 update 中调用）
pub fn show_window(ctx: &egui::Context, app: &mut FileSyncApp) {
    // 加载中提示
    if matches!(app.preview_state, PreviewState::Loading) {
        egui::Window::new(t("🔍 预览扫描中…", "🔍 Scanning…"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t(
                        "正在扫描文件差异，请稍候…",
                        "Scanning for differences, please wait…",
                    ));
                });
                if ui.button(t("取消", "Cancel")).clicked() {
                    app.preview_state = PreviewState::Idle;
                }
            });
        return;
    }

    // 错误提示
    if let PreviewState::Error(ref msg) = app.preview_state.clone() {
        egui::Window::new(t("预览失败", "Preview Failed"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(msg).color(egui::Color32::RED));
                if ui.button(t("关闭", "Close")).clicked() {
                    app.preview_state = PreviewState::Idle;
                }
            });
        return;
    }

    // 结果窗口
    if let PreviewState::Ready(ref entries) = app.preview_state.clone() {
        let mut to_copy = Vec::new();
        let mut to_skip = Vec::new();
        let mut orphans = Vec::new();
        for e in entries {
            match e.action {
                DiffAction::Create | DiffAction::Update => to_copy.push(e),
                DiffAction::Skip => to_skip.push(e),
                DiffAction::Orphan => orphans.push(e),
            }
        }

        let total_bytes: u64 = to_copy.iter().map(|e| e.size).sum();

        // 当前任务的同步模式（在触发预览扫描时确定，避免 selected_job 偏移引发的误判）
        let is_mirror = app.preview_job_is_mirror;

        // 按主窗口尺寸计算弹窗大小，留出窗口边框空间（各边 ~10px）
        let screen = ctx.screen_rect();
        let win_w = (screen.width()  * 0.80).clamp(400.0, screen.width()  - 20.0);
        let win_h = (screen.height() * 0.72).clamp(260.0, screen.height() - 20.0);
        let scroll_h = (screen.height() * 0.48).max(160.0);

        let mut open = true;
        egui::Window::new(t("🔍 同步预览", "🔍 Sync Preview"))
            .id(egui::Id::new("preview_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)   // fixed_size 每帧精确控制，不依赖 egui 存储的旧尺寸
            .fixed_size([win_w, win_h])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 强制约束内容区最大宽度，防止长文件名撑开窗口
                ui.set_max_width(win_w);

                // 摘要行
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(if is_zh() {
                            format!(
                                "▶ 复制 {} 个文件（{}）",
                                to_copy.len(),
                                fmt_bytes(total_bytes)
                            )
                        } else {
                            format!(
                                "▶ Copy {} file(s)  ({})",
                                to_copy.len(),
                                fmt_bytes(total_bytes)
                            )
                        })
                        .color(egui::Color32::from_rgb(100, 200, 255))
                        .strong(),
                    );
                    ui.separator();
                    ui.label(if is_zh() {
                        format!("跳过 {} 个", to_skip.len())
                    } else {
                        format!("Skip {}", to_skip.len())
                    });
                    if !orphans.is_empty() {
                        ui.separator();
                        if is_mirror {
                            ui.label(
                                egui::RichText::new(if is_zh() {
                                    format!("删除 {} 个孤立文件", orphans.len())
                                } else {
                                    format!("Delete {} orphan(s)", orphans.len())
                                })
                                .color(egui::Color32::from_rgb(255, 100, 100)),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(if is_zh() {
                                    format!("孤立 {} 个（不删除）", orphans.len())
                                } else {
                                    format!("{} orphan(s) (kept)", orphans.len())
                                })
                                .color(ui.visuals().weak_text_color()),
                            );
                        }
                    }
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                // 只显示需要操作的文件：排除 Skip；
                // 增量模式下不删除孤立文件，也不列出（避免误导用户）。
                let action_entries: Vec<_> = entries
                    .iter()
                    .filter(|e| {
                        e.action != DiffAction::Skip
                            && (is_mirror || e.action != DiffAction::Orphan)
                    })
                    .collect();
                let show_entries: Vec<_> = action_entries.iter().take(1000).collect();
                let truncated = action_entries.len() > 1000;

                // 固定列宽
                let action_w = 50.0;
                let time_w = 140.0;
                let size_w = 80.0;
                let row_h = ui.spacing().interact_size.y;
                let name_w = (win_w - action_w - time_w - size_w - 24.0).max(100.0);

                // 表头
                ui.horizontal(|ui| {
                    ui.set_max_width(win_w);
                    ui.add_sized(
                        [action_w, row_h],
                        egui::Label::new(
                            egui::RichText::new(t("操作", "Action")).strong(),
                        ),
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(name_w, row_h),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(name_w);
                            ui.add(egui::Label::new(
                                egui::RichText::new(t("名称", "Name")).strong(),
                            ));
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(time_w, row_h),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(time_w);
                            ui.add(egui::Label::new(
                                egui::RichText::new(t("修改日期", "Modified")).strong(),
                            ));
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(size_w, row_h),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(size_w);
                            ui.add(egui::Label::new(
                                egui::RichText::new(t("大小", "Size")).strong(),
                            ));
                        },
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("preview_scroll")
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        for entry in &show_entries {
                            let (action_text, color) = match entry.action {
                                DiffAction::Create => (
                                    t("新增", "New"),
                                    egui::Color32::from_rgb(80, 200, 120),
                                ),
                                DiffAction::Update => (
                                    t("覆盖", "Update"),
                                    egui::Color32::from_rgb(100, 180, 255),
                                ),
                                DiffAction::Skip => unreachable!(),
                                DiffAction::Orphan => (
                                    t("删除", "Delete"),
                                    egui::Color32::from_rgb(255, 100, 100),
                                ),
                            };
                            let time_str: String = DateTime::<Local>::from(entry.modified)
                                .format("%Y-%m-%d %H:%M")
                                .to_string();
                            let size_str = fmt_bytes(entry.size);
                            let full_path = entry.relative_path.to_string_lossy();
                            ui.horizontal(|ui| {
                                ui.set_max_width(win_w);
                                ui.add_sized(
                                    [action_w, row_h],
                                    egui::Label::new(
                                        egui::RichText::new(action_text).color(color),
                                    ),
                                );
                                ui.allocate_ui_with_layout(
                                    egui::vec2(name_w, row_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(name_w);
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(full_path.as_ref()).monospace(),
                                            )
                                            .truncate(),
                                        )
                                    },
                                )
                                .response
                                .on_hover_text(&*full_path);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(time_w, row_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(time_w);
                                        ui.add(egui::Label::new(
                                            egui::RichText::new(&time_str)
                                                .color(ui.visuals().weak_text_color()),
                                        ));
                                    },
                                );
                                ui.allocate_ui_with_layout(
                                    egui::vec2(size_w, row_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(size_w);
                                        ui.add(egui::Label::new(
                                            egui::RichText::new(&size_str)
                                                .color(ui.visuals().weak_text_color()),
                                        ));
                                    },
                                );
                            });
                        }

                        if truncated {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(if is_zh() {
                                    format!("… 仅显示前 1000 条，共 {} 条", action_entries.len())
                                } else {
                                    format!(
                                        "… Showing first 1000 of {} entries",
                                        action_entries.len()
                                    )
                                })
                                .color(ui.visuals().weak_text_color()),
                            );
                        }
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !app.sync_running,
                            egui::Button::new(t("▶ 立即同步", "▶ Sync Now")),
                        )
                        .clicked()
                    {
                        app.preview_state = PreviewState::Idle;
                        app.save();
                        app.start_sync(ctx);
                    }
                    ui.add_space(8.0);
                    if ui.button(t("关闭", "Close")).clicked() {
                        app.preview_state = PreviewState::Idle;
                    }
                });
            });

        if !open {
            app.preview_state = PreviewState::Idle;
        }
    }
}

fn fmt_bytes(bytes: u64) -> String {
    super::fmt_bytes(bytes)
}
