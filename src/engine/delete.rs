use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::engine::events::DeleteFallbackChoice;
use crate::engine::interaction::SyncInteraction;
use crate::model::job::{DeleteFallbackPolicy, DeleteMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    Skipped,
}

pub fn delete_with_mode(
    path: &Path,
    mode: &DeleteMode,
    fallback_policy: &DeleteFallbackPolicy,
    interaction: &dyn SyncInteraction,
    is_dir: bool,
    stop: &Arc<AtomicBool>,
) -> Result<DeleteOutcome, String> {
    match mode {
        DeleteMode::Direct => delete_direct(path).map(|_| DeleteOutcome::Deleted),
        DeleteMode::RecycleBin => trash::delete(path)
            .map(|_| DeleteOutcome::Deleted)
            .map_err(|e| e.to_string()),
        DeleteMode::FollowSystem => match trash::delete(path) {
            Ok(()) => Ok(DeleteOutcome::Deleted),
            Err(e) => request_delete_confirmation(
                path,
                fallback_policy,
                interaction,
                is_dir,
                e.to_string(),
                stop,
            ),
        },
    }
}

pub fn request_delete_confirmation(
    path: &Path,
    fallback_policy: &DeleteFallbackPolicy,
    interaction: &dyn SyncInteraction,
    is_dir: bool,
    reason: String,
    stop: &Arc<AtomicBool>,
) -> Result<DeleteOutcome, String> {
    if stop.load(Ordering::Relaxed) {
        return Err(stopped_error().to_string());
    }

    match fallback_policy {
        DeleteFallbackPolicy::Skip => return Ok(DeleteOutcome::Skipped),
        DeleteFallbackPolicy::Fail => {
            return Err(recycle_bin_failed_message(&reason));
        }
        DeleteFallbackPolicy::Ask => {}
    }

    if !interaction.allows_prompts() {
        return Err(unattended_prompt_denied_message(&reason));
    }

    let item_label = if is_dir { "directory" } else { "file" };
    match interaction.request_delete_fallback(
        path,
        is_dir,
        format!("Failed to move {} to Recycle Bin: {}", item_label, reason),
    ) {
        DeleteFallbackChoice::DirectDelete => delete_direct(path).map(|_| DeleteOutcome::Deleted),
        DeleteFallbackChoice::Skip => Ok(DeleteOutcome::Skipped),
        DeleteFallbackChoice::StopSync => Err("stopped".into()),
    }
}

pub fn delete_direct(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

pub fn delete_failed_message(is_dir: bool, err: &str) -> String {
    if crate::i18n::is_zh() {
        if is_dir {
            format!("\u{5220}\u{9664}\u{5B64}\u{7ACB}\u{76EE}\u{5F55}\u{5931}\u{8D25}: {}", err)
        } else {
            format!("\u{5220}\u{9664}\u{5B64}\u{7ACB}\u{6587}\u{4EF6}\u{5931}\u{8D25}: {}", err)
        }
    } else if is_dir {
        format!("Failed to delete orphan directory: {}", err)
    } else {
        format!("Failed to delete orphan file: {}", err)
    }
}

fn recycle_bin_failed_message(reason: &str) -> String {
    if crate::i18n::is_zh() {
        format!("\u{56DE}\u{6536}\u{7AD9}\u{5220}\u{9664}\u{5931}\u{8D25}: {}", reason)
    } else {
        format!("Recycle Bin delete failed: {}", reason)
    }
}

fn unattended_prompt_denied_message(reason: &str) -> String {
    if crate::i18n::is_zh() {
        format!(
            "\u{56DE}\u{6536}\u{7AD9}\u{5220}\u{9664}\u{5931}\u{8D25}\u{4E14}\u{5F53}\u{524D}\u{4E3A}\u{65E0}\u{4EBA}\u{503C}\u{5B88}\u{8FD0}\u{884C}\u{FF0C}\u{65E0}\u{6CD5}\u{7B49}\u{5F85}\u{786E}\u{8BA4}: {}",
            reason
        )
    } else {
        format!(
            "Recycle Bin delete failed during unattended execution, so confirmation is unavailable: {}",
            reason
        )
    }
}

fn stopped_error() -> &'static str {
    if crate::i18n::is_zh() {
        "\u{5DF2}\u{505C}\u{6B62}"
    } else {
        "stopped"
    }
}

#[cfg(test)]
mod tests {
    use super::{delete_direct, delete_failed_message, request_delete_confirmation, DeleteOutcome};
    use crate::engine::events::DeleteFallbackChoice;
    use crate::engine::interaction::SyncInteraction;
    use crate::model::job::DeleteFallbackPolicy;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    struct MockInteraction {
        allow_prompts: bool,
        fallback_choice: DeleteFallbackChoice,
    }

    impl SyncInteraction for MockInteraction {
        fn allows_prompts(&self) -> bool {
            self.allow_prompts
        }

        fn confirm_mass_delete(&self, _count: u64) -> bool {
            true
        }

        fn request_delete_fallback(
            &self,
            _path: &Path,
            _is_dir: bool,
            _message: String,
        ) -> DeleteFallbackChoice {
            self.fallback_choice
        }
    }

    #[test]
    fn follow_system_delete_requires_confirmation_before_direct_delete() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("orphan.txt");
        std::fs::write(&path, b"data").unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let interaction = MockInteraction {
            allow_prompts: true,
            fallback_choice: DeleteFallbackChoice::Skip,
        };

        let result = request_delete_confirmation(
            &path,
            &DeleteFallbackPolicy::Ask,
            &interaction,
            false,
            "Recycle Bin unavailable".into(),
            &stop,
        )
        .unwrap();

        assert_eq!(result, DeleteOutcome::Skipped);
        assert!(path.exists());
    }

    #[test]
    fn fallback_policy_skip_keeps_file_without_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("keep.txt");
        std::fs::write(&path, b"data").unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let interaction = MockInteraction {
            allow_prompts: false,
            fallback_choice: DeleteFallbackChoice::DirectDelete,
        };

        let result = request_delete_confirmation(
            &path,
            &DeleteFallbackPolicy::Skip,
            &interaction,
            false,
            "Recycle Bin unavailable".into(),
            &stop,
        )
        .unwrap();

        assert_eq!(result, DeleteOutcome::Skipped);
        assert!(path.exists());
    }

    #[test]
    fn unattended_prompt_request_becomes_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("blocked.txt");
        std::fs::write(&path, b"data").unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let interaction = MockInteraction {
            allow_prompts: false,
            fallback_choice: DeleteFallbackChoice::Skip,
        };

        let err = request_delete_confirmation(
            &path,
            &DeleteFallbackPolicy::Ask,
            &interaction,
            false,
            "Recycle Bin unavailable".into(),
            &stop,
        )
        .unwrap_err();

        assert!(err.contains("\u{65E0}\u{4EBA}\u{503C}\u{5B88}") || err.contains("unattended"));
        assert!(path.exists());
    }

    #[test]
    fn direct_delete_removes_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("remove.txt");
        std::fs::write(&path, b"data").unwrap();

        delete_direct(&path).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn delete_error_message_is_scope_specific() {
        let file_msg = delete_failed_message(false, "Access denied");
        let dir_msg = delete_failed_message(true, "Access denied");

        assert_ne!(file_msg, dir_msg);
    }
}
