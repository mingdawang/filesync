pub fn scan_source_failed(err: &str) -> String {
    crate::messages::scan_source_failed(err)
}

pub fn scan_destination_failed(err: &str) -> String {
    crate::messages::scan_destination_failed(err)
}

pub fn create_destination_failed(err: &str) -> String {
    crate::messages::create_destination_failed(err)
}

pub fn source_directory_skipped() -> &'static str {
    crate::messages::source_directory_skipped()
}

pub fn mirror_delete_cancelled(count: u64, threshold: u64) -> String {
    crate::messages::mirror_delete_cancelled(count, threshold)
}

pub fn scheduled_mirror_delete_blocked(count: u64, threshold: u64) -> String {
    crate::messages::scheduled_mirror_delete_blocked(count, threshold)
}

pub fn copy_task_panic(err: &str) -> String {
    crate::messages::copy_task_panic(err)
}

pub fn delete_task_panic(err: &str) -> String {
    crate::messages::delete_task_panic(err)
}

pub fn recycle_bin_prompt(item_label: &str, reason: &str) -> String {
    crate::messages::recycle_bin_prompt(item_label, reason)
}
