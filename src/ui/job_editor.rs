use std::path::PathBuf;

use egui::Ui;

use crate::app::FileSyncApp;
use crate::i18n::{is_zh, t};
use crate::model::config::CompareMethod;
use crate::model::job::{ExclusionRule, FolderPair, SyncMode};

pub fn show(ui: &mut Ui, app: &mut FileSyncApp) {
    let Some(idx) = app.selected_job else {
        show_welcome(ui);
        return;
    };

    if idx >= app.config.jobs.len() {
        app.selected_job = None;
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("job_editor_scroll")
        .show(ui, |ui| {
            // 同步进行中禁止修改配置
            if app.sync_running {
                ui.disable();
            }

            ui.add_space(8.0);

            // ── 任务名称 ──────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(t("任务名称:", "Job Name:"));
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut app.config.jobs[idx].name)
                            .desired_width(280.0),
                    )
                    .changed()
                {
                    app.dirty = true;
                }
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            show_folder_pairs(ui, app, idx);

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            show_exclusions(ui, app, idx);

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            // ── 同步模式 ──────────────────────────────────────────────
            ui.strong(t("同步模式", "Sync Mode"));
            ui.add_space(4.0);

            let mode = &mut app.config.jobs[idx].sync_mode;
            let mut changed = false;

            ui.horizontal(|ui| {
                if ui.radio(*mode == SyncMode::Update, t("增量更新", "Update")).clicked() {
                    *mode = SyncMode::Update;
                    changed = true;
                }
                ui.label(
                    egui::RichText::new(t(
                        "复制新增/变更文件，保留目标端多余文件",
                        "Copy new/changed files, keep extra files on destination",
                    ))
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            });
            ui.horizontal(|ui| {
                if ui.radio(*mode == SyncMode::Mirror, t("镜像同步", "Mirror")).clicked() {
                    *mode = SyncMode::Mirror;
                    changed = true;
                }
                ui.label(
                    egui::RichText::new(t(
                        "复制新增/变更文件，删除目标端孤立文件",
                        "Copy new/changed files, delete orphan files on destination",
                    ))
                    .small()
                    .color(egui::Color32::from_rgb(255, 180, 80)),
                );
            });
            if changed {
                app.dirty = true;
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            // ── 文件比较方式 ──────────────────────────────────────────
            ui.strong(t("文件比较方式", "File Comparison"));
            ui.add_space(4.0);

            let cm = &mut app.config.jobs[idx].compare_method;
            let mut cm_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .radio(*cm == CompareMethod::Metadata, t("元数据（大小 + 时间）", "Metadata (size + mtime)"))
                    .clicked()
                {
                    *cm = CompareMethod::Metadata;
                    cm_changed = true;
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .radio(*cm == CompareMethod::Hash, t("内容哈希（BLAKE3，精确但较慢）", "Content hash (BLAKE3, accurate but slower)"))
                    .clicked()
                {
                    *cm = CompareMethod::Hash;
                    cm_changed = true;
                }
            });
            if cm_changed {
                app.dirty = true;
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            // ── 并发数 ────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(t("并发数:", "Concurrency:"));
                let mut c = app.config.jobs[idx].concurrency;
                if ui
                    .add(
                        egui::Slider::new(&mut c, 1usize..=16)
                            .text(t("线程", "threads")),
                    )
                    .changed()
                {
                    app.config.jobs[idx].concurrency = c;
                    app.dirty = true;
                }
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            show_schedule(ui, app, idx);

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            // ── 高级选项 ──────────────────────────────────────────────
            ui.collapsing(t("高级选项", "Advanced"), |ui| {
                ui.add_space(4.0);

                // Delta 传输阈值
                ui.horizontal(|ui| {
                    ui.label(t("差量传输阈值:", "Delta threshold:"));
                    let mut mb =
                        app.config.jobs[idx].engine_options.delta_threshold_mb as usize;
                    if ui
                        .add(egui::Slider::new(&mut mb, 0usize..=512).suffix(" MB"))
                        .changed()
                    {
                        app.config.jobs[idx].engine_options.delta_threshold_mb = mb as u64;
                        app.dirty = true;
                    }
                });
                ui.label(
                    egui::RichText::new(t(
                        "0 = 禁用差量传输；建议 4 MB 以上文件才启用",
                        "0 = disable delta; recommended for files > 4 MB",
                    ))
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );

                ui.add_space(6.0);

                // 无缓冲 IO 阈值
                ui.horizontal(|ui| {
                    ui.label(t("无缓冲 IO 阈值:", "Unbuffered I/O threshold:"));
                    let mut mb =
                        app.config.jobs[idx].engine_options.unbuffered_threshold_mb as usize;
                    if ui
                        .add(egui::Slider::new(&mut mb, 64usize..=1024).suffix(" MB"))
                        .changed()
                    {
                        app.config.jobs[idx].engine_options.unbuffered_threshold_mb = mb as u64;
                        app.dirty = true;
                    }
                });
                ui.label(
                    egui::RichText::new(t(
                        "大于此值的文件绕过系统缓存，适合大文件拷贝",
                        "Files above this size bypass system cache",
                    ))
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );

                ui.add_space(6.0);

                // 复制后校验
                if ui
                    .checkbox(
                        &mut app.config.jobs[idx].engine_options.verify_after_copy,
                        t("复制后 BLAKE3 校验", "Verify after copy (BLAKE3)"),
                    )
                    .on_hover_text(t(
                        "每个文件复制完成后对比源与目标的哈希值，确保数据完整（会增加额外耗时）",
                        "Compare source and destination hash after each copy to ensure integrity (adds extra time)",
                    ))
                    .changed()
                {
                    app.dirty = true;
                }
            });

            ui.add_space(16.0);

            // ── 操作按钮 ──────────────────────────────────────────────
            ui.horizontal(|ui| {
                let save_text = if app.dirty {
                    t("💾 保存*", "💾 Save*")
                } else {
                    t("💾 保存", "💾 Save")
                };
                if ui.button(save_text).clicked() {
                    if let Some(err) = app.validate_folder_pairs_for_save(idx) {
                        app.error_message = Some(err);
                        return;
                    }
                    app.save();
                }
                ui.add_space(8.0);
                if ui
                    .add_enabled(!app.sync_running, egui::Button::new(t("🔍 同步预览", "🔍 Sync Preview")))
                    .clicked()
                {
                    if let Some(err) = app.validate_folder_pairs_for_start(idx) {
                        app.error_message = Some(err);
                    } else {
                        app.save();
                        app.start_preview(ui.ctx());
                    }
                }
            });

            ui.add_space(8.0);
        });
}

// ─────────────────────────────────────────────────────────────────
// 文件夹对编辑区
// ─────────────────────────────────────────────────────────────────

fn show_folder_pairs(ui: &mut Ui, app: &mut FileSyncApp, job_idx: usize) {
    ui.horizontal(|ui| {
        ui.strong(t("文件夹对", "Folder Pairs"));
        if ui.small_button(t("＋ 添加", "＋ Add")).clicked() {
            app.config.jobs[job_idx].folder_pairs.push(FolderPair::new());
            app.dirty = true;
        }
    });

    ui.add_space(6.0);

    let pair_count = app.config.jobs[job_idx].folder_pairs.len();
    let mut to_remove: Option<usize> = None;
    let mut to_move_up: Option<usize> = None;
    let mut to_move_down: Option<usize> = None;

    for i in 0..pair_count {
        let cur_source = app.config.jobs[job_idx].folder_pairs[i]
            .source
            .to_string_lossy()
            .into_owned();
        let cur_dest = app.config.jobs[job_idx].folder_pairs[i]
            .destination
            .to_string_lossy()
            .into_owned();
        let cur_enabled = app.config.jobs[job_idx].folder_pairs[i].enabled;

        let mut source_str = cur_source.clone();
        let mut dest_str = cur_dest.clone();
        let mut enabled = cur_enabled;
        let mut picked_source: Option<PathBuf> = None;
        let mut picked_dest: Option<PathBuf> = None;
        let mut delete_this = false;
        let mut move_up_this = false;
        let mut move_down_this = false;

        egui::Frame::group(ui.style()).show(ui, |ui| {
            // 头部行：复选框 + 右侧操作按钮
            ui.horizontal(|ui| {
                ui.checkbox(&mut enabled, format!("# {}", i + 1));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("🗑")
                        .on_hover_text(t("删除此对", "Delete this pair"))
                        .clicked()
                    {
                        delete_this = true;
                    }
                    if i + 1 < pair_count
                        && ui
                            .small_button("↓")
                            .on_hover_text(t("下移", "Move Down"))
                            .clicked()
                    {
                        move_down_this = true;
                    }
                    if i > 0
                        && ui
                            .small_button("↑")
                            .on_hover_text(t("上移", "Move Up"))
                            .clicked()
                    {
                        move_up_this = true;
                    }
                });
            });

            // 源目录行
            if let Some(p) = path_row(
                ui,
                t("源:  ", "Src: "),
                &mut source_str,
                t("源文件夹路径（或将文件夹拖拽到此）", "Source folder path (or drag & drop)"),
            ) {
                picked_source = Some(p);
            }

            // 目标目录行
            if let Some(p) = path_row(
                ui,
                t("目的:", "Dst:"),
                &mut dest_str,
                t("目标文件夹路径（或将文件夹拖拽到此）", "Destination folder path (or drag & drop)"),
            ) {
                picked_dest = Some(p);
            }
        });

        // 回写变更
        {
            let pair = &mut app.config.jobs[job_idx].folder_pairs[i];
            let mut changed = false;

            if pair.enabled != enabled {
                pair.enabled = enabled;
                changed = true;
            }
            if let Some(p) = picked_source {
                pair.source = p;
                changed = true;
            } else if source_str != cur_source {
                pair.source = PathBuf::from(&source_str);
                changed = true;
            }
            if let Some(p) = picked_dest {
                pair.destination = p;
                changed = true;
            } else if dest_str != cur_dest {
                pair.destination = PathBuf::from(&dest_str);
                changed = true;
            }
            if changed {
                app.dirty = true;
            }
        }

        if delete_this {
            to_remove = Some(i);
        }
        if move_up_this {
            to_move_up = Some(i);
        }
        if move_down_this {
            to_move_down = Some(i);
        }

        ui.add_space(4.0);
    }

    if let Some(i) = to_remove {
        app.config.jobs[job_idx].folder_pairs.remove(i);
        app.dirty = true;
    } else if let Some(i) = to_move_up {
        app.config.jobs[job_idx].folder_pairs.swap(i, i - 1);
        app.dirty = true;
    } else if let Some(i) = to_move_down {
        app.config.jobs[job_idx].folder_pairs.swap(i, i + 1);
        app.dirty = true;
    }
}

// ─────────────────────────────────────────────────────────────────
// 排除规则编辑区
// ─────────────────────────────────────────────────────────────────

fn show_exclusions(ui: &mut Ui, app: &mut FileSyncApp, job_idx: usize) {
    ui.strong(t("排除规则", "Exclusion Rules"));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(t(
            "支持 Glob 模式：*.tmp  .git/**  node_modules/**",
            "Supports Glob patterns: *.tmp  .git/**  node_modules/**",
        ))
        .small()
        .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(6.0);

    let rule_count = app.config.jobs[job_idx].exclusions.len();
    let mut to_remove: Option<usize> = None;
    let mut toggle: Option<usize> = None;

    ui.horizontal_wrapped(|ui| {
        for i in 0..rule_count {
            let rule = &app.config.jobs[job_idx].exclusions[i];
            let (bg, fg) = if rule.enabled {
                (
                    egui::Color32::from_rgb(30, 80, 160),
                    egui::Color32::from_rgb(160, 200, 255),
                )
            } else {
                (egui::Color32::from_gray(50), egui::Color32::from_gray(140))
            };

            egui::Frame::none()
                .fill(bg)
                .rounding(egui::Rounding::same(4.0))
                .inner_margin(egui::Margin::symmetric(6.0, 2.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let resp = ui.label(egui::RichText::new(&rule.pattern).color(fg).small());
                        if resp
                            .on_hover_text(t("点击启用/禁用", "Click to enable/disable"))
                            .clicked()
                        {
                            toggle = Some(i);
                        }
                        if ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(" ×")
                                        .color(egui::Color32::from_gray(180))
                                        .small(),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text(t("删除规则", "Delete rule"))
                            .clicked()
                        {
                            to_remove = Some(i);
                        }
                    });
                });
        }
    });

    if let Some(i) = toggle {
        app.config.jobs[job_idx].exclusions[i].enabled =
            !app.config.jobs[job_idx].exclusions[i].enabled;
        app.dirty = true;
    }
    if let Some(i) = to_remove {
        app.config.jobs[job_idx].exclusions.remove(i);
        app.dirty = true;
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut app.new_exclusion_input)
                .hint_text(t("输入 Glob 规则后按 Enter", "Enter Glob pattern and press Enter"))
                .desired_width(220.0),
        );

        let confirm = ui.button(t("添加", "Add")).clicked()
            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));

        if confirm {
            let pattern = app.new_exclusion_input.trim().to_string();
            if !pattern.is_empty() {
                match globset::GlobBuilder::new(&pattern)
                    .case_insensitive(true)
                    .build()
                {
                    Ok(_) => {
                        app.config.jobs[job_idx]
                            .exclusions
                            .push(ExclusionRule::new(pattern));
                        app.new_exclusion_input.clear();
                        app.exclusion_error = None;
                        app.dirty = true;
                    }
                    Err(e) => {
                        app.exclusion_error = Some(if is_zh() {
                            format!("无效的 Glob 规则: {}", e)
                        } else {
                            format!("Invalid Glob pattern: {}", e)
                        });
                    }
                }
            }
        }

        if resp.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            app.exclusion_error = None;
        }
    });

    if let Some(err) = &app.exclusion_error {
        ui.label(egui::RichText::new(err).color(egui::Color32::RED).small());
    }
}

// ─────────────────────────────────────────────────────────────────
// 定时同步配置区
// ─────────────────────────────────────────────────────────────────

fn show_schedule(ui: &mut Ui, app: &mut FileSyncApp, idx: usize) {
    ui.strong(t("定时同步", "Scheduled Sync"));
    ui.add_space(4.0);

    if ui
        .checkbox(
            &mut app.config.jobs[idx].schedule.enabled,
            t("启用定时同步", "Enable scheduled sync"),
        )
        .changed()
    {
        if app.config.jobs[idx].schedule.enabled {
            // Validate: need at least one enabled folder pair with both paths set
            let has_valid_pair = app.config.jobs[idx].folder_pairs.iter().any(|p| {
                p.enabled
                    && !p.source.as_os_str().is_empty()
                    && !p.destination.as_os_str().is_empty()
            });
            if !has_valid_pair {
                app.config.jobs[idx].schedule.enabled = false;
                app.error_message = Some(
                    t(
                        "请先配置至少一个已启用且源/目标路径均已填写的文件夹对。",
                        "Please configure at least one enabled folder pair with source and destination paths.",
                    )
                    .into(),
                );
                return;
            }
        }
        app.dirty = true;
    }

    if !app.config.jobs[idx].schedule.enabled {
        return;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(t("同步间隔:", "Interval:"));
        let mut mins = app.config.jobs[idx].schedule.interval_minutes as usize;
        if ui
            .add(
                egui::Slider::new(&mut mins, 5usize..=1440)
                    .suffix(t(" 分钟", " min")),
            )
            .on_hover_text(t(
                "最小 5 分钟，最大 1440 分钟（1 天）",
                "Min 5 minutes, max 1440 minutes (1 day)",
            ))
            .changed()
        {
            app.config.jobs[idx].schedule.interval_minutes = mins as u32;
            app.dirty = true;
        }
    });

    let mins = app.config.jobs[idx].schedule.interval_minutes;
    let interval_desc = if is_zh() {
        if mins < 60 {
            format!("每 {} 分钟", mins)
        } else if mins % 60 == 0 {
            format!("每 {} 小时", mins / 60)
        } else {
            format!("每 {} 小时 {} 分钟", mins / 60, mins % 60)
        }
    } else {
        if mins < 60 {
            format!("Every {} min", mins)
        } else if mins % 60 == 0 {
            format!("Every {}h", mins / 60)
        } else {
            format!("Every {}h {}min", mins / 60, mins % 60)
        }
    };

    let next_info = match app.config.jobs[idx].last_sync_time {
        Some(last) => {
            let next = last
                + chrono::Duration::minutes(
                    app.config.jobs[idx].schedule.interval_minutes as i64,
                );
            let next_local = next.with_timezone(&chrono::Local);
            let now = chrono::Local::now();
            if next_local <= now {
                t("下次同步：即将触发", "Next sync: imminent").to_string()
            } else {
                let time = next_local.format("%m-%d %H:%M");
                if is_zh() {
                    format!("下次同步：{}", time)
                } else {
                    format!("Next sync: {}", time)
                }
            }
        }
        None => t(
            "下次同步：就绪（从未运行，将在下次检查时立即触发）",
            "Next sync: ready (never run, triggers on next check)",
        )
        .to_string(),
    };

    ui.label(
        egui::RichText::new(format!("{}  ·  {}", interval_desc, next_info))
            .small()
            .color(ui.visuals().weak_text_color()),
    );
}

// ─────────────────────────────────────────────────────────────────
// 欢迎页（无任务选中）
// ─────────────────────────────────────────────────────────────────

fn show_welcome(ui: &mut Ui) {
    let available = ui.available_size();
    ui.add_space(available.y * 0.25);
    ui.vertical_centered(|ui| {
        ui.heading("FileSync");
        ui.add_space(8.0);
        ui.label(t("高性能文件夹同步工具", "High-performance folder sync tool"));
        ui.add_space(16.0);
        ui.label(
            egui::RichText::new(t(
                "← 点击左侧「＋ 新建任务」开始",
                "← Click \"＋ New Job\" on the left to get started",
            ))
            .color(ui.visuals().weak_text_color()),
        );
    });
}

// ─────────────────────────────────────────────────────────────────
// 辅助
// ─────────────────────────────────────────────────────────────────

/// 渲染一行路径输入（标签 + 文本框 + 浏览按钮 + 资源管理器按钮 + 拖放支持）。
///
/// 文本框内容通过 `path_str` 原地修改；浏览或拖放选中的路径通过返回值传出。
fn path_row(ui: &mut Ui, label: &str, path_str: &mut String, hint: &str) -> Option<PathBuf> {
    let mut picked: Option<PathBuf> = None;
    let current = path_str.clone();
    ui.horizontal(|ui| {
        ui.label(label);
        let resp = ui.add(
            egui::TextEdit::singleline(path_str)
                .desired_width(ui.available_width() - 100.0)
                .hint_text(hint),
        );
        if ui.button(t("浏览", "Browse")).clicked() {
            let dir = if current.is_empty() { "." } else { current.as_str() };
            if let Some(p) = rfd::FileDialog::new().set_directory(dir).pick_folder() {
                picked = Some(p);
            }
        }
        let cur_path = std::path::Path::new(path_str.as_str());
        if ui
            .add_enabled(cur_path.exists(), egui::Button::new("📂"))
            .on_hover_text(t("在资源管理器中打开", "Open in Explorer"))
            .clicked()
        {
            open_in_explorer(cur_path);
        }
        if resp.hovered() {
            if let Some(p) = get_dropped_folder(ui) {
                picked = Some(p);
            }
        }
    });
    picked
}

fn open_in_explorer(path: &std::path::Path) {
    let target = if path.exists() {
        path.to_path_buf()
    } else {
        path.ancestors()
            .find(|a| a.exists())
            .map(|a| a.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    let _ = std::process::Command::new("explorer.exe").arg(target).spawn();
}

fn get_dropped_folder(ui: &egui::Ui) -> Option<std::path::PathBuf> {
    ui.input(|i| {
        i.raw.dropped_files.iter().find_map(|f| {
            let p = f.path.as_ref()?;
            if p.is_dir() {
                Some(p.clone())
            } else {
                p.parent().map(|pp| pp.to_path_buf())
            }
        })
    })
}
