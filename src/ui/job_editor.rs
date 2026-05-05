use std::path::PathBuf;

use egui::Ui;

use crate::app::FileSyncApp;
use crate::i18n::{is_zh, t};
use crate::model::config::CompareMethod;
use crate::model::job::{
    DeleteFallbackPolicy, DeleteMode, ExclusionRule, FolderPair, ReliabilityMode, RunResultStatus,
    RunTrigger, SyncMode,
};

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
                    app.config.jobs[idx].dirty = true;
                }
            });

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t("模板:", "Templates:")).small());
                if ui.small_button(t("文档日常", "Daily Docs")).clicked() {
                    apply_job_template(&mut app.config.jobs[idx], JobTemplate::DailyDocs);
                }
                if ui.small_button(t("照片备份", "Photo Backup")).clicked() {
                    apply_job_template(&mut app.config.jobs[idx], JobTemplate::PhotoBackup);
                }
                if ui.small_button(t("镜像归档", "Mirror Archive")).clicked() {
                    apply_job_template(&mut app.config.jobs[idx], JobTemplate::MirrorArchive);
                }
            });
            ui.label(
                egui::RichText::new(t(
                    "模板会直接填入推荐策略，不改动现有源/目标路径。",
                    "Templates apply recommended strategy presets without changing current paths.",
                ))
                .small()
                .color(ui.visuals().weak_text_color()),
            );

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
                app.config.jobs[idx].dirty = true;
            }

            if app.config.jobs[idx].sync_mode == SyncMode::Mirror {
                ui.label(
                    egui::RichText::new(t(
                        "风险提示：镜像同步会清理目标端孤立文件，适合备份盘和无人值守任务，但不适合临时试运行。",
                        "Risk: Mirror sync cleans destination orphans. Good for backups and unattended jobs, not for casual trial runs.",
                    ))
                    .small()
                    .color(egui::Color32::from_rgb(255, 170, 80)),
                );
            }

            if app.config.jobs[idx].sync_mode == SyncMode::Mirror {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                ui.strong(t("删除方式", "Delete Mode"));
                ui.add_space(4.0);

                let delete_mode = &mut app.config.jobs[idx].delete_mode;
                let mut delete_changed = false;

                ui.horizontal(|ui| {
                    if ui
                        .radio(*delete_mode == DeleteMode::Direct, t("直接删除", "Direct delete"))
                        .clicked()
                    {
                        *delete_mode = DeleteMode::Direct;
                        delete_changed = true;
                    }
                    ui.label(
                        egui::RichText::new(t(
                            "不经过回收站，直接从目标端删除",
                            "Delete from destination without using Recycle Bin",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                });
                ui.horizontal(|ui| {
                    if ui
                        .radio(*delete_mode == DeleteMode::RecycleBin, t("放入回收站", "Recycle Bin"))
                        .clicked()
                    {
                        *delete_mode = DeleteMode::RecycleBin;
                        delete_changed = true;
                    }
                    ui.label(
                        egui::RichText::new(t(
                            "必须放入回收站；失败时记为错误，不直接删除",
                            "Must move to Recycle Bin; failure is reported as an error",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                });
                ui.horizontal(|ui| {
                    if ui
                        .radio(*delete_mode == DeleteMode::FollowSystem, t("跟随系统", "Follow system"))
                        .clicked()
                    {
                        *delete_mode = DeleteMode::FollowSystem;
                        delete_changed = true;
                    }
                    ui.label(
                        egui::RichText::new(t(
                            "优先放入回收站；如果失败，会弹出确认，再决定是否直接删除",
                            "Prefer Recycle Bin; if it fails, ask before deleting directly",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                });
                if *delete_mode == DeleteMode::FollowSystem {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(t(
                            "回收站失败时",
                            "If Recycle Bin delete fails",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                    let fallback = &mut app.config.jobs[idx].delete_fallback_policy;
                    ui.horizontal(|ui| {
                        if ui
                            .radio(*fallback == DeleteFallbackPolicy::Ask, t("询问", "Ask"))
                            .clicked()
                        {
                            *fallback = DeleteFallbackPolicy::Ask;
                            delete_changed = true;
                        }
                        if ui
                            .radio(*fallback == DeleteFallbackPolicy::Skip, t("跳过", "Skip"))
                            .clicked()
                        {
                            *fallback = DeleteFallbackPolicy::Skip;
                            delete_changed = true;
                        }
                        if ui
                            .radio(*fallback == DeleteFallbackPolicy::Fail, t("记为失败", "Fail"))
                            .clicked()
                        {
                            *fallback = DeleteFallbackPolicy::Fail;
                            delete_changed = true;
                        }
                    });
                    ui.label(
                        egui::RichText::new(t(
                            "提示：定时/无人值守运行时，“询问”不会等待确认，而是直接记为失败。",
                            "Note: for scheduled/unattended runs, \"Ask\" will fail immediately instead of waiting.",
                        ))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                }

                if delete_changed {
                    app.config.jobs[idx].dirty = true;
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            // ── 可靠性模式 ────────────────────────────────────────────
            ui.strong(t("可靠性模式", "Reliability Mode"));
            ui.add_space(4.0);

            let current_reliability_mode = app.config.jobs[idx].reliability_mode.clone();
            let mut selected_reliability_mode = None;
            let mut reliability_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .radio(current_reliability_mode == ReliabilityMode::Fast, t("极速", "Fast"))
                    .clicked()
                {
                    selected_reliability_mode = Some(ReliabilityMode::Fast);
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .radio(current_reliability_mode == ReliabilityMode::Balanced, t("平衡", "Balanced"))
                    .clicked()
                {
                    selected_reliability_mode = Some(ReliabilityMode::Balanced);
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .radio(current_reliability_mode == ReliabilityMode::Safe, t("安全", "Safe"))
                    .clicked()
                {
                    selected_reliability_mode = Some(ReliabilityMode::Safe);
                }
            });
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(match current_reliability_mode {
                        ReliabilityMode::Fast => t(
                            "元数据比较，不做复制后校验，适合速度优先。",
                            "Metadata compare, no post-copy verification, best for speed.",
                        ),
                        ReliabilityMode::Balanced => t(
                            "内容哈希比较，不做复制后校验，兼顾正确性与效率。",
                            "Content-hash compare without post-copy verification, balanced for accuracy and speed.",
                        ),
                        ReliabilityMode::Safe => t(
                            "内容哈希比较，并在复制后再次校验，适合重要数据。",
                            "Content-hash compare plus post-copy verification, best for critical data.",
                        ),
                        ReliabilityMode::Custom => t(
                            "当前为自定义组合，可在高级选项中单独调整。",
                            "Currently using a custom combination; adjust it in Advanced settings.",
                        ),
                    })
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            });
            if let Some(mode) = selected_reliability_mode {
                apply_reliability_mode(&mut app.config.jobs[idx], mode);
                reliability_changed = true;
            }
            if reliability_changed {
                app.config.jobs[idx].dirty = true;
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
                    app.config.jobs[idx].dirty = true;
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
                    app.config.jobs[idx].reliability_mode = ReliabilityMode::Custom;
                    app.config.jobs[idx].dirty = true;
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
                    app.config.jobs[idx].reliability_mode = ReliabilityMode::Custom;
                    app.config.jobs[idx].dirty = true;
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
                    app.config.jobs[idx].reliability_mode = ReliabilityMode::Custom;
                    app.config.jobs[idx].dirty = true;
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                ui.strong(t("比较方式（高级）", "Comparison (Advanced)"));
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
                    app.config.jobs[idx].reliability_mode = ReliabilityMode::Custom;
                    app.config.jobs[idx].dirty = true;
                }
            });

            if !app.config.jobs[idx].run_history.is_empty() {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                ui.collapsing(t("最近运行", "Recent Runs"), |ui| {
                    for entry in app.config.jobs[idx].run_history.iter().take(10) {
                        let trigger = match entry.trigger {
                            RunTrigger::Manual => t("手动", "Manual"),
                            RunTrigger::Scheduled => t("定时", "Scheduled"),
                            RunTrigger::Retry => t("重试", "Retry"),
                        };
                        let result = match entry.result {
                            RunResultStatus::Completed => t("成功", "Success"),
                            RunResultStatus::Warning => t("告警", "Warning"),
                            RunResultStatus::Failed => t("失败", "Failed"),
                            RunResultStatus::Stopped => t("停止", "Stopped"),
                            RunResultStatus::Missed => t("漏跑", "Missed"),
                        };
                        let line = if entry.note.is_empty() {
                            format!(
                                "{}  [{} / {}]",
                                entry.finished_at.with_timezone(&chrono::Local).format("%m-%d %H:%M"),
                                trigger,
                                result
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
                        ui.label(
                            egui::RichText::new(line)
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            }

            ui.add_space(16.0);

            // ── 操作按钮 ──────────────────────────────────────────────
            ui.horizontal(|ui| {
                let save_text = if app.current_job_dirty() {
                    t("💾 保存*", "💾 Save*")
                } else {
                    t("💾 保存", "💾 Save")
                };
                if ui.button(save_text).clicked() {
                    app.save_job_with_validation(idx);
                }
                ui.add_space(8.0);
                if ui
                    .add_enabled(!app.sync_running, egui::Button::new(t("🔍 同步预览", "🔍 Sync Preview")))
                    .clicked()
                {
                    app.start_preview_with_validation(idx, ui.ctx());
                }
            });

            ui.add_space(8.0);
        });
}

fn apply_reliability_mode(job: &mut crate::model::job::SyncJob, mode: ReliabilityMode) {
    job.reliability_mode = mode.clone();
    match mode {
        ReliabilityMode::Fast => {
            job.compare_method = CompareMethod::Metadata;
            job.engine_options.verify_after_copy = false;
        }
        ReliabilityMode::Balanced => {
            job.compare_method = CompareMethod::Hash;
            job.engine_options.verify_after_copy = false;
        }
        ReliabilityMode::Safe => {
            job.compare_method = CompareMethod::Hash;
            job.engine_options.verify_after_copy = true;
        }
        ReliabilityMode::Custom => {}
    }
}

enum JobTemplate {
    DailyDocs,
    PhotoBackup,
    MirrorArchive,
}

fn apply_job_template(job: &mut crate::model::job::SyncJob, template: JobTemplate) {
    match template {
        JobTemplate::DailyDocs => {
            job.sync_mode = SyncMode::Update;
            apply_reliability_mode(job, ReliabilityMode::Balanced);
            job.concurrency = job.concurrency.clamp(2, 4);
            job.schedule.enabled = true;
            job.schedule.interval_minutes = 60;
            job.schedule.retry_on_failure = true;
            job.schedule.max_retries = 2;
            job.schedule.retry_delay_minutes = 10;
        }
        JobTemplate::PhotoBackup => {
            job.sync_mode = SyncMode::Update;
            apply_reliability_mode(job, ReliabilityMode::Safe);
            job.concurrency = job.concurrency.clamp(1, 3);
            job.schedule.enabled = true;
            job.schedule.interval_minutes = 180;
            job.schedule.retry_on_failure = true;
            job.schedule.max_retries = 3;
            job.schedule.retry_delay_minutes = 15;
        }
        JobTemplate::MirrorArchive => {
            job.sync_mode = SyncMode::Mirror;
            job.delete_mode = DeleteMode::FollowSystem;
            job.delete_fallback_policy = DeleteFallbackPolicy::Fail;
            apply_reliability_mode(job, ReliabilityMode::Safe);
            job.schedule.enabled = true;
            job.schedule.interval_minutes = 1440;
            job.schedule.retry_on_failure = true;
            job.schedule.max_retries = 2;
            job.schedule.retry_delay_minutes = 30;
        }
    }
    job.dirty = true;
}

// ─────────────────────────────────────────────────────────────────
// 文件夹对编辑区
// ─────────────────────────────────────────────────────────────────

fn show_folder_pairs(ui: &mut Ui, app: &mut FileSyncApp, job_idx: usize) {
    ui.horizontal(|ui| {
        ui.strong(t("文件夹对", "Folder Pairs"));
        if ui.small_button(t("＋ 添加", "＋ Add")).clicked() {
            app.config.jobs[job_idx].folder_pairs.push(FolderPair::new());
            app.config.jobs[job_idx].dirty = true;
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
                app.config.jobs[job_idx].dirty = true;
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
        app.config.jobs[job_idx].dirty = true;
    } else if let Some(i) = to_move_up {
        app.config.jobs[job_idx].folder_pairs.swap(i, i - 1);
        app.config.jobs[job_idx].dirty = true;
    } else if let Some(i) = to_move_down {
        app.config.jobs[job_idx].folder_pairs.swap(i, i + 1);
        app.config.jobs[job_idx].dirty = true;
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
        app.config.jobs[job_idx].dirty = true;
    }
    if let Some(i) = to_remove {
        app.config.jobs[job_idx].exclusions.remove(i);
        app.config.jobs[job_idx].dirty = true;
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
                        app.config.jobs[job_idx].dirty = true;
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
            if !app.job_has_valid_enabled_folder_pair(idx) {
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
        app.config.jobs[idx].dirty = true;
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
            app.config.jobs[idx].dirty = true;
        }
    });

    ui.add_space(6.0);
    if ui
        .checkbox(
            &mut app.config.jobs[idx].schedule.retry_on_failure,
            t("失败后自动重试", "Retry automatically after failure"),
        )
        .changed()
    {
        app.config.jobs[idx].dirty = true;
    }

    if app.config.jobs[idx].schedule.retry_on_failure {
        ui.horizontal(|ui| {
            ui.label(t("最多重试:", "Max retries:"));
            let mut retries = app.config.jobs[idx].schedule.max_retries as usize;
            if ui.add(egui::Slider::new(&mut retries, 1usize..=5)).changed() {
                app.config.jobs[idx].schedule.max_retries = retries as u8;
                app.config.jobs[idx].dirty = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label(t("重试间隔:", "Retry delay:"));
            let mut mins = app.config.jobs[idx].schedule.retry_delay_minutes as usize;
            if ui
                .add(egui::Slider::new(&mut mins, 1usize..=120).suffix(t(" 分钟", " min")))
                .changed()
            {
                app.config.jobs[idx].schedule.retry_delay_minutes = mins as u32;
                app.config.jobs[idx].dirty = true;
            }
        });
    }

    ui.horizontal(|ui| {
        ui.label(t("连续失败后暂停:", "Pause after failures:"));
        let mut count = app.config.jobs[idx].schedule.pause_after_failures as usize;
        if ui.add(egui::Slider::new(&mut count, 1usize..=10)).changed() {
            app.config.jobs[idx].schedule.pause_after_failures = count as u8;
            app.config.jobs[idx].dirty = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label(t("删除异常阈值:", "Delete threshold:"));
        let mut count = app.config.jobs[idx].schedule.delete_threshold as usize;
        if ui.add(egui::Slider::new(&mut count, 10usize..=10000)).changed() {
            app.config.jobs[idx].schedule.delete_threshold = count as u64;
            app.config.jobs[idx].dirty = true;
        }
    });

    if app.config.jobs[idx].sync_mode == SyncMode::Mirror {
        if ui
            .checkbox(
                &mut app.config.jobs[idx].schedule.risk_acknowledged,
                t(
                    "已确认镜像定时任务会删除目标端孤立文件",
                    "I understand scheduled mirror sync deletes destination orphans",
                ),
            )
            .changed()
        {
            app.config.jobs[idx].dirty = true;
        }
    }

    if app.config.jobs[idx].schedule.paused {
        let pause_text = if app.config.jobs[idx].schedule.pause_reason.is_empty() {
            t("当前定时任务已暂停。", "This scheduled task is currently paused.").to_string()
        } else {
            app.config.jobs[idx].schedule.pause_reason.clone()
        };
        ui.label(
            egui::RichText::new(pause_text)
                .small()
                .color(egui::Color32::from_rgb(255, 170, 80)),
        );
        if ui.button(t("恢复定时任务", "Resume Schedule")).clicked() {
            app.config.jobs[idx].schedule.paused = false;
            app.config.jobs[idx].schedule.pause_reason.clear();
            app.config.jobs[idx].schedule.consecutive_failures = 0;
            app.config.jobs[idx].dirty = true;
        }
    }

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
