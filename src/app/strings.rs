use crate::i18n::t;
use crate::model::job::{RunResultStatus, RunSummary, RunTrigger};

pub(super) fn error_title() -> &'static str {
    t("\u{9519}\u{8BEF}", "Error")
}

pub(super) fn ok_button() -> &'static str {
    "OK"
}

pub(super) fn delete_confirmation_title(is_dir: bool) -> &'static str {
    if is_dir {
        t("\u{76EE}\u{5F55}\u{5220}\u{9664}\u{786E}\u{8BA4}", "Directory Delete Confirmation")
    } else {
        t("\u{6587}\u{4EF6}\u{5220}\u{9664}\u{786E}\u{8BA4}", "File Delete Confirmation")
    }
}

pub(super) fn delete_fallback_body(path: &str, message: &str) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{65E0}\u{6CD5}\u{5C06}\u{8BE5}\u{9879}\u{79FB}\u{5165}\u{56DE}\u{6536}\u{7AD9}:\n{}\n\n{}\n\n\u{662F}\u{5426}\u{7EE7}\u{7EED}\u{76F4}\u{63A5}\u{5220}\u{9664}\u{FF1F}",
            path, message
        )
    } else {
        format!(
            "Failed to move this item to the Recycle Bin:\n{}\n\n{}\n\nDo you want to continue with direct delete?",
            path, message
        )
    }
}

pub(super) fn delete_directly_button() -> &'static str {
    t("\u{76F4}\u{63A5}\u{5220}\u{9664}", "Delete Directly")
}

pub(super) fn skip_button() -> &'static str {
    t("\u{8DF3}\u{8FC7}", "Skip")
}

pub(super) fn stop_sync_button() -> &'static str {
    t("\u{505C}\u{6B62}\u{540C}\u{6B65}", "Stop Sync")
}

pub(super) fn mass_delete_title() -> &'static str {
    t("\u{6279}\u{91CF}\u{5220}\u{9664}\u{786E}\u{8BA4}", "Mass Delete Confirmation")
}

pub(super) fn mass_delete_body(count: u64) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{672C}\u{6B21}\u{955C}\u{50CF}\u{540C}\u{6B65}\u{5C06}\u{5220}\u{9664} {} \u{4E2A}\u{76EE}\u{6807}\u{9879}\u{76EE}\u{FF0C}\u{5DF2}\u{8D85}\u{8FC7}\u{5B89}\u{5168}\u{9608}\u{503C}\u{3002}\n\n\u{662F}\u{5426}\u{7EE7}\u{7EED}\u{FF1F}",
            count
        )
    } else {
        format!(
            "This mirror sync is about to delete {} destination items, exceeding the safety threshold.\n\nContinue?",
            count
        )
    }
}

pub(super) fn continue_button() -> &'static str {
    t("\u{7EE7}\u{7EED}", "Continue")
}

pub(super) fn cancel_this_run_button() -> &'static str {
    t("\u{53D6}\u{6D88}\u{672C}\u{6B21}\u{8FD0}\u{884C}", "Cancel This Run")
}

pub(super) fn risk_confirmation_title() -> &'static str {
    t("\u{98CE}\u{9669}\u{786E}\u{8BA4}", "Risk Confirmation")
}

pub(super) fn mirror_delete_warning() -> &'static str {
    t(
        "\u{955C}\u{50CF}\u{540C}\u{6B65}\u{4F1A}\u{5220}\u{9664}\u{76EE}\u{6807}\u{7AEF}\u{5B64}\u{7ACB}\u{6587}\u{4EF6}\u{3002}",
        "Mirror sync deletes orphan files on destination.",
    )
}

pub(super) fn direct_delete_warning() -> &'static str {
    t(
        "\u{5F53}\u{524D}\u{5220}\u{9664}\u{7B56}\u{7565}\u{4E3A}\u{76F4}\u{63A5}\u{5220}\u{9664}\u{FF0C}\u{4E0D}\u{7ECF}\u{8FC7}\u{56DE}\u{6536}\u{7AD9}\u{3002}",
        "Delete mode is direct delete, without Recycle Bin.",
    )
}

pub(super) fn cancel_button() -> &'static str {
    t("\u{53D6}\u{6D88}", "Cancel")
}

pub(super) fn task_history_title() -> &'static str {
    t("\u{4EFB}\u{52A1}\u{5386}\u{53F2}", "Task History")
}

pub(super) fn trigger_label(trigger: RunTrigger) -> &'static str {
    match trigger {
        RunTrigger::Manual => t("\u{624B}\u{52A8}", "Manual"),
        RunTrigger::Scheduled => t("\u{5B9A}\u{65F6}", "Scheduled"),
        RunTrigger::Retry => t("\u{91CD}\u{8BD5}", "Retry"),
    }
}

pub(super) fn result_label(result: RunResultStatus) -> &'static str {
    match result {
        RunResultStatus::Completed => t("\u{6210}\u{529F}", "Success"),
        RunResultStatus::Warning => t("\u{8B66}\u{544A}", "Warning"),
        RunResultStatus::Failed => t("\u{5931}\u{8D25}", "Failed"),
        RunResultStatus::Stopped => t("\u{505C}\u{6B62}", "Stopped"),
        RunResultStatus::Missed => t("\u{672A}\u{6267}\u{884C}", "Missed"),
    }
}

pub(super) fn history_summary_line(
    finished_at: String,
    trigger: RunTrigger,
    result: RunResultStatus,
    summary: &RunSummary,
    note: &str,
) -> String {
    if crate::i18n::is_zh() {
        format!(
            "{}  [{} / {}]  \u{590D}\u{5236} {}  \u{8DF3}\u{8FC7} {}  \u{9519}\u{8BEF} {}  \u{5220}\u{9664} {}  {}",
            finished_at,
            trigger_label(trigger),
            result_label(result),
            summary.copied,
            summary.skipped,
            summary.errors,
            summary.deleted,
            note
        )
    } else {
        format!(
            "{}  [{} / {}]  copied {}  skipped {}  errors {}  deleted {}  {}",
            finished_at,
            trigger_label(trigger),
            result_label(result),
            summary.copied,
            summary.skipped,
            summary.errors,
            summary.deleted,
            note
        )
    }
}

pub(super) fn history_note_line(
    finished_at: String,
    trigger: RunTrigger,
    result: RunResultStatus,
    note: &str,
) -> String {
    format!(
        "{}  [{} / {}]  {}",
        finished_at,
        trigger_label(trigger),
        result_label(result),
        note
    )
}

pub(super) fn unsaved_changes_title() -> &'static str {
    t("\u{672A}\u{4FDD}\u{5B58}\u{7684}\u{4FEE}\u{6539}", "Unsaved Changes")
}

pub(super) fn unsaved_changes_body() -> &'static str {
    t(
        "\u{5F53}\u{524D}\u{6709}\u{672A}\u{4FDD}\u{5B58}\u{7684}\u{4FEE}\u{6539}\u{FF0C}\u{9000}\u{51FA}\u{524D}\u{662F}\u{5426}\u{4FDD}\u{5B58}\u{FF1F}",
        "You have unsaved changes. Save before quitting?",
    )
}

pub(super) fn save_button() -> &'static str {
    t("\u{4FDD}\u{5B58}", "Save")
}

pub(super) fn dont_save_button() -> &'static str {
    t("\u{4E0D}\u{4FDD}\u{5B58}", "Don't Save")
}

pub(super) fn close_title() -> &'static str {
    t("\u{5173}\u{95ED} FileSync", "Close FileSync")
}

pub(super) fn close_running_warning() -> &'static str {
    t(
        "\u{540C}\u{6B65}\u{6B63}\u{5728}\u{8FDB}\u{884C}\u{4E2D}\u{FF0C}\u{9000}\u{51FA}\u{5C06}\u{4E2D}\u{65AD}\u{5F53}\u{524D}\u{540C}\u{6B65}\u{3002}",
        "Sync is in progress. Quitting will interrupt it.",
    )
}

pub(super) fn choose_close_action() -> &'static str {
    t("\u{8BF7}\u{9009}\u{62E9}\u{5173}\u{95ED}\u{884C}\u{4E3A}\u{FF1A}", "Choose what to do:")
}

pub(super) fn minimize_to_tray_button() -> &'static str {
    t("\u{6700}\u{5C0F}\u{5316}\u{5230}\u{6258}\u{76D8}", "Minimize to Tray")
}

pub(super) fn quit_button() -> &'static str {
    t("\u{9000}\u{51FA}", "Quit")
}

pub(super) fn remember_close_choice() -> &'static str {
    t(
        "\u{8BB0}\u{4F4F}\u{6211}\u{7684}\u{9009}\u{62E9}\u{FF08}\u{53EF}\u{5728}\u{8BBE}\u{7F6E}\u{4E2D}\u{4FEE}\u{6539}\u{FF09}",
        "Remember my choice (can be changed in Settings)",
    )
}

pub(super) fn top_unsaved_changes() -> &'static str {
    t("\u{672A}\u{4FDD}\u{5B58}\u{4FEE}\u{6539}", "Unsaved changes")
}

pub(super) fn about_button() -> &'static str {
    t("\u{5173}\u{4E8E}", "About")
}

pub(super) fn settings_button() -> &'static str {
    t("\u{8BBE}\u{7F6E}", "Settings")
}

pub(super) fn history_button() -> &'static str {
    t("\u{5386}\u{53F2}", "History")
}

pub(super) fn shortcuts_hint() -> &'static str {
    "Ctrl+S Save  F5 Sync"
}

pub(super) fn settings_title() -> &'static str {
    t("\u{8BBE}\u{7F6E}", "Settings")
}

pub(super) fn theme_title() -> &'static str {
    t("\u{4E3B}\u{9898}", "Theme")
}

pub(super) fn follow_system_theme() -> &'static str {
    t("\u{8DDF}\u{968F}\u{7CFB}\u{7EDF}", "Follow System")
}

pub(super) fn light_theme() -> &'static str {
    t("\u{6D45}\u{8272}", "Light")
}

pub(super) fn dark_theme() -> &'static str {
    t("\u{6DF1}\u{8272}", "Dark")
}

pub(super) fn default_concurrency_title() -> &'static str {
    t("\u{65B0}\u{4EFB}\u{52A1}\u{9ED8}\u{8BA4}\u{5E76}\u{53D1}\u{6570}", "Default Concurrency for New Jobs")
}

pub(super) fn threads_label() -> &'static str {
    t("\u{7EBF}\u{7A0B}", "threads")
}

pub(super) fn close_action_title() -> &'static str {
    t(
        "\u{70B9}\u{51FB}\u{5173}\u{95ED}\u{6309}\u{94AE}\u{FF08}X\u{FF09}\u{65F6}",
        "When clicking Close (X)",
    )
}

pub(super) fn ask_every_time() -> &'static str {
    t("\u{6BCF}\u{6B21}\u{8BE2}\u{95EE}", "Ask every time")
}

pub(super) fn config_backup_title() -> &'static str {
    t("\u{914D}\u{7F6E}\u{5907}\u{4EFD}", "Config Backup")
}

pub(super) fn export_config_button() -> &'static str {
    t("\u{5BFC}\u{51FA}\u{914D}\u{7F6E}", "Export Config")
}

pub(super) fn import_config_button() -> &'static str {
    t("\u{5BFC}\u{5165}\u{914D}\u{7F6E}", "Import Config")
}

pub(super) fn import_overwrite_warning() -> &'static str {
    t(
        "\u{5BFC}\u{5165}\u{5C06}\u{8986}\u{76D6}\u{5F53}\u{524D}\u{6240}\u{6709}\u{4EFB}\u{52A1}\u{548C}\u{8BBE}\u{7F6E}",
        "Import will overwrite all current jobs and settings",
    )
}

pub(super) fn about_title() -> &'static str {
    t("\u{5173}\u{4E8E} FileSync", "About FileSync")
}

pub(super) fn about_subtitle() -> &'static str {
    t("\u{9AD8}\u{6027}\u{80FD}\u{6587}\u{4EF6}\u{5939}\u{540C}\u{6B65}\u{5DE5}\u{5177}", "High-performance folder sync tool")
}

pub(super) fn version_label() -> &'static str {
    t("\u{7248}\u{672C}", "Version")
}

pub(super) fn toolchain_label() -> &'static str {
    t("\u{5DE5}\u{5177}\u{94FE}", "Toolchain")
}

pub(super) fn platform_label() -> &'static str {
    t("\u{5E73}\u{53F0}", "Platform")
}

pub(super) fn config_path_label() -> &'static str {
    t("\u{914D}\u{7F6E}\u{8DEF}\u{5F84}", "Config path")
}

pub(super) fn about_features() -> &'static str {
    t(
        "NTFS/ReFS USN \u{52A0}\u{901F}\u{3001}Delta \u{540C}\u{6B65}\u{3001}CopyFileEx \u{652F}\u{6301}",
        "NTFS/ReFS USN acceleration, Delta sync, and CopyFileEx support",
    )
}

pub(super) fn export_config_dialog_title() -> &'static str {
    export_config_button()
}

pub(super) fn import_config_dialog_title() -> &'static str {
    import_config_button()
}

pub(super) fn json_config_filter() -> &'static str {
    t("JSON \u{914D}\u{7F6E}", "JSON config")
}

pub(super) fn notification_success_icon() -> &'static str {
    t("\u{6210}\u{529F}", "OK")
}

pub(super) fn notification_warning_icon() -> &'static str {
    t("\u{8B66}\u{544A}", "WARN")
}

pub(super) fn close_overlay_button() -> &'static str {
    "\u{00D7}"
}

pub(super) fn sync_log_write_failed(err: &str) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{540C}\u{6B65}\u{5DF2}\u{5B8C}\u{6210}\u{FF0C}\u{4F46}\u{5199}\u{5165}\u{65E5}\u{5FD7}\u{5931}\u{8D25}: {}",
            err
        )
    } else {
        format!("Sync completed, but writing the log failed: {}", err)
    }
}

pub(super) fn stopped_on_user_request() -> &'static str {
    t(
        "\u{8FD0}\u{884C}\u{5DF2}\u{6309}\u{7528}\u{6237}\u{8BF7}\u{6C42}\u{505C}\u{6B62}\u{3002}",
        "Run stopped on user request.",
    )
}

pub(super) fn retry_scheduled(next_attempt: u32, ready_at: &str) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{5DF2}\u{5B89}\u{6392}\u{7B2C} {} \u{6B21}\u{91CD}\u{8BD5}\u{FF0C}\u{6267}\u{884C}\u{65F6}\u{95F4} {}",
            next_attempt, ready_at
        )
    } else {
        format!("Retry {} scheduled for {}.", next_attempt, ready_at)
    }
}

pub(super) fn run_completed_with_errors() -> &'static str {
    t(
        "\u{672C}\u{6B21}\u{8FD0}\u{884C}\u{5B8C}\u{6210}\u{4F46}\u{5B58}\u{5728}\u{9519}\u{8BEF}\u{FF0C}\u{8BF7}\u{68C0}\u{67E5}\u{65E5}\u{5FD7}\u{3002}",
        "This run completed with errors. Review the error log below.",
    )
}

pub(super) fn failed_to_start_sync(message: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{542F}\u{52A8}\u{540C}\u{6B65}\u{5931}\u{8D25}: {}", message)
    } else {
        format!("Failed to start sync: {}", message)
    }
}

pub(super) fn disk_full_sync_stopped() -> &'static str {
    t(
        "\u{78C1}\u{76D8}\u{7A7A}\u{95F4}\u{4E0D}\u{8DB3}\u{FF0C}\u{540C}\u{6B65}\u{5DF2}\u{505C}\u{6B62}\u{3002}",
        "Disk full, sync stopped.",
    )
}

pub(super) fn scheduled_sync_paused_after_failures(failures: u32, note: &str) -> String {
    if note.is_empty() {
        if crate::i18n::is_zh() {
            format!(
                "\u{8FDE}\u{7EED}\u{5931}\u{8D25} {} \u{6B21}\u{FF0C}\u{5DF2}\u{6682}\u{505C}\u{5B9A}\u{65F6}\u{4EFB}\u{52A1}\u{3002}",
                failures
            )
        } else {
            format!("Scheduled sync paused after {} consecutive failures.", failures)
        }
    } else if crate::i18n::is_zh() {
        format!(
            "\u{8FDE}\u{7EED}\u{5931}\u{8D25} {} \u{6B21}\u{FF0C}\u{5DF2}\u{6682}\u{505C}\u{5B9A}\u{65F6}\u{4EFB}\u{52A1}\u{3002} {}",
            failures, note
        )
    } else {
        format!(
            "Scheduled sync paused after {} consecutive failures. {}",
            failures, note
        )
    }
}
