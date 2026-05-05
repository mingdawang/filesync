use std::collections::HashMap;

use crate::model::job::UsnCheckpoint;
use crate::model::runtime::JobStateRecord;

use super::FileSyncApp;

impl FileSyncApp {
    pub fn is_dirty(&self) -> bool {
        self.settings_dirty || self.job_transient.values().any(|state| state.dirty)
    }

    pub fn current_job_dirty(&self) -> bool {
        self.selected_job
            .map(|idx| {
                self.config
                    .jobs
                    .get(idx)
                    .and_then(|job| self.job_transient.get(&job.id))
                    .map(|state| state.dirty)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub fn job_state(&self, job_id: uuid::Uuid) -> Option<&JobStateRecord> {
        self.config.job_states.iter().find(|state| state.job_id == job_id)
    }

    pub fn job_state_mut(&mut self, job_id: uuid::Uuid) -> Option<&mut JobStateRecord> {
        self.config
            .job_states
            .iter_mut()
            .find(|state| state.job_id == job_id)
    }

    pub fn ensure_job_state_mut(&mut self, job_id: uuid::Uuid) -> &mut JobStateRecord {
        if let Some(idx) = self
            .config
            .job_states
            .iter()
            .position(|state| state.job_id == job_id)
        {
            return &mut self.config.job_states[idx];
        }
        self.config.job_states.push(JobStateRecord {
            job_id,
            ..JobStateRecord::default()
        });
        self.config.job_states.last_mut().unwrap()
    }

    pub fn mark_job_dirty(&mut self, job_id: uuid::Uuid) {
        self.job_transient.entry(job_id).or_default().dirty = true;
    }

    pub fn clear_job_dirty(&mut self, job_id: uuid::Uuid) {
        self.job_transient.entry(job_id).or_default().dirty = false;
    }

    pub fn job_checkpoints(&self, job_id: uuid::Uuid) -> HashMap<String, UsnCheckpoint> {
        self.job_transient
            .get(&job_id)
            .map(|state| state.last_sync_checkpoints.clone())
            .unwrap_or_default()
    }
}
