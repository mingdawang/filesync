use crate::i18n::t;

use super::FileSyncApp;

impl FileSyncApp {
    pub fn validate_folder_pairs_for_save(&self, idx: usize) -> Option<String> {
        if self.job_has_partial_enabled_folder_pair(idx) {
            Some(
                t(
                    "存在已启用但源/目标路径不完整的文件夹对，请检查配置后再保存。",
                    "Some enabled folder pairs have incomplete paths. Please fix them before saving.",
                )
                .into(),
            )
        } else {
            None
        }
    }

    pub fn validate_folder_pairs_for_start(&self, idx: usize) -> Option<String> {
        if self.job_has_partial_enabled_folder_pair(idx) {
            return Some(
                t(
                    "存在已启用但源/目标路径不完整的文件夹对，请检查配置。",
                    "Some enabled folder pairs have incomplete paths. Please fix them.",
                )
                .into(),
            );
        }
        if !self.job_has_valid_enabled_folder_pair(idx) {
            Some(
                t(
                    "请先配置至少一个已启用且源/目标路径均已填写的文件夹对。",
                    "Please configure at least one enabled folder pair with source and destination paths.",
                )
                .into(),
            )
        } else {
            None
        }
    }

    pub fn job_has_partial_enabled_folder_pair(&self, idx: usize) -> bool {
        self.config
            .jobs
            .get(idx)
            .is_some_and(|job| has_partial_enabled_folder_pair(&job.folder_pairs))
    }

    pub fn job_has_valid_enabled_folder_pair(&self, idx: usize) -> bool {
        self.config
            .jobs
            .get(idx)
            .is_some_and(|job| has_valid_enabled_folder_pair(&job.folder_pairs))
    }

    pub fn save_job_with_validation(&mut self, idx: usize) -> bool {
        if let Some(err) = self.validate_folder_pairs_for_save(idx) {
            self.error_message = Some(err);
            return false;
        }
        self.save();
        true
    }
}

pub(super) fn has_partial_enabled_folder_pair(
    folder_pairs: &[crate::model::job::FolderPair],
) -> bool {
    folder_pairs.iter().any(|pair| {
        pair.enabled
            && (pair.source.as_os_str().is_empty()
                != pair.destination.as_os_str().is_empty())
    })
}

pub(super) fn has_valid_enabled_folder_pair(
    folder_pairs: &[crate::model::job::FolderPair],
) -> bool {
    folder_pairs.iter().any(|pair| {
        pair.enabled
            && !pair.source.as_os_str().is_empty()
            && !pair.destination.as_os_str().is_empty()
    })
}
