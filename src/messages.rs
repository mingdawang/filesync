pub fn source_not_found(path: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{6E90}\u{76EE}\u{5F55}\u{4E0D}\u{5B58}\u{5728}: {}", path)
    } else {
        format!("Source directory not found: {}", path)
    }
}

pub fn scan_source_failed(err: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{626B}\u{63CF}\u{6E90}\u{76EE}\u{5F55}\u{5931}\u{8D25}: {}", err)
    } else {
        format!("Failed to scan source directory: {}", err)
    }
}

pub fn scan_destination_failed(err: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{626B}\u{63CF}\u{76EE}\u{6807}\u{76EE}\u{5F55}\u{5931}\u{8D25}: {}", err)
    } else {
        format!("Failed to scan destination directory: {}", err)
    }
}

pub fn create_destination_failed(err: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{521B}\u{5EFA}\u{76EE}\u{6807}\u{76EE}\u{5F55}\u{5931}\u{8D25}: {}", err)
    } else {
        format!("Failed to create destination directories: {}", err)
    }
}

pub fn source_scan_issue(count: usize, first_message: &str) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{6E90}\u{76EE}\u{5F55}\u{626B}\u{63CF}\u{53D1}\u{73B0} {} \u{4E2A}\u{95EE}\u{9898}\u{FF0C}\u{9996}\u{4E2A}\u{95EE}\u{9898}: {}",
            count, first_message
        )
    } else {
        format!("Source scan found {} issue(s); first issue: {}", count, first_message)
    }
}

pub fn destination_scan_issue(count: usize, first_message: &str) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{76EE}\u{6807}\u{76EE}\u{5F55}\u{626B}\u{63CF}\u{53D1}\u{73B0} {} \u{4E2A}\u{95EE}\u{9898}\u{FF0C}\u{9996}\u{4E2A}\u{95EE}\u{9898}: {}",
            count, first_message
        )
    } else {
        format!(
            "Destination scan found {} issue(s); first issue: {}",
            count, first_message
        )
    }
}

pub fn source_directory_skipped() -> &'static str {
    if crate::i18n::is_zh() {
        "\u{6E90}\u{76EE}\u{5F55}\u{4E0D}\u{5B58}\u{5728}\u{FF0C}\u{5DF2}\u{8DF3}\u{8FC7}"
    } else {
        "Source directory missing; skipped"
    }
}

pub fn mirror_delete_cancelled(count: u64, threshold: u64) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{955C}\u{50CF}\u{5220}\u{9664}\u{5DF2}\u{53D6}\u{6D88}: {} \u{4E2A}\u{9879}\u{76EE}\u{8D85}\u{8FC7}\u{9608}\u{503C} {}",
            count, threshold
        )
    } else {
        format!(
            "Mirror delete cancelled: {} items exceed threshold {}",
            count, threshold
        )
    }
}

pub fn scheduled_mirror_delete_blocked(count: u64, threshold: u64) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{65E0}\u{4EBA}\u{503C}\u{5B88}\u{7684}\u{955C}\u{50CF}\u{5220}\u{9664}\u{5DF2}\u{963B}\u{6B62}: {} \u{4E2A}\u{9879}\u{76EE}\u{8D85}\u{8FC7}\u{9608}\u{503C} {}",
            count, threshold
        )
    } else {
        format!(
            "Scheduled mirror delete blocked: {} items exceed threshold {}",
            count, threshold
        )
    }
}

pub fn copy_task_panic(err: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{590D}\u{5236}\u{4EFB}\u{52A1}\u{5D29}\u{6E83}: {}", err)
    } else {
        format!("Copy task panic: {}", err)
    }
}

pub fn delete_task_panic(err: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{5220}\u{9664}\u{4EFB}\u{52A1}\u{5D29}\u{6E83}: {}", err)
    } else {
        format!("Delete task panic: {}", err)
    }
}

pub fn recycle_bin_prompt(item_label: &str, reason: &str) -> String {
    format!("Failed to move {} to Recycle Bin: {}", item_label, reason)
}

#[allow(dead_code)]
pub fn completion_notification_title(job_name: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{300C}{}\u{300D}\u{540C}\u{6B65}\u{5B8C}\u{6210}", job_name)
    } else {
        format!("\"{}\" sync complete", job_name)
    }
}

#[allow(dead_code)]
pub fn completion_notification_body(
    copied: u64,
    skipped: u64,
    errors: u64,
    deleted: u64,
) -> String {
    let mut parts = if crate::i18n::is_zh() {
        vec![
            format!("\u{590D}\u{5236} {}", copied),
            format!("\u{8DF3}\u{8FC7} {}", skipped),
        ]
    } else {
        vec![format!("Copied {}", copied), format!("Skipped {}", skipped)]
    };

    if errors > 0 {
        parts.push(if crate::i18n::is_zh() {
            format!("\u{9519}\u{8BEF} {}", errors)
        } else {
            format!("Errors {}", errors)
        });
    }
    if deleted > 0 {
        parts.push(if crate::i18n::is_zh() {
            format!("\u{5220}\u{9664} {}", deleted)
        } else {
            format!("Deleted {}", deleted)
        });
    }

    parts.join("  ")
}
