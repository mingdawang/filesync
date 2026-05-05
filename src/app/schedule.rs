use chrono::{DateTime, Utc};
use eframe::egui;

use crate::app::{flow, QueueEntry};
use crate::model::config::AppConfig;
use crate::model::job::{RunHistoryEntry, RunResultStatus, RunTrigger, SyncMode};

use super::{is_schedule_due, FileSyncApp};

impl FileSyncApp {
    pub(super) fn start_pending_queued_job(&mut self, ctx: &egui::Context) {
        if !self.pending_queue_start || self.sync_running {
            return;
        }

        let Some(entry) = self.job_queue.front().cloned() else {
            self.pending_queue_start = false;
            return;
        };

        if entry.ready_at > Utc::now() {
            let wait = (entry.ready_at - Utc::now())
                .to_std()
                .unwrap_or_else(|_| std::time::Duration::from_secs(1));
            ctx.request_repaint_after(wait.min(std::time::Duration::from_secs(30)));
            return;
        }

        let entry = self.job_queue.pop_front().unwrap();
        self.pending_queue_start = !self.job_queue.is_empty();

        let Some(job_idx) = self.find_job_idx_by_id(entry.job_id) else {
            self.start_pending_queued_job(ctx);
            return;
        };

        if let Some(err) = self.validate_folder_pairs_for_start(job_idx) {
            flow::record_run_history(
                self,
                job_idx,
                RunHistoryEntry {
                    started_at: Utc::now(),
                    finished_at: Utc::now(),
                    trigger: entry.trigger,
                    result: RunResultStatus::Missed,
                    retry_attempt: entry.retry_attempt,
                    summary: None,
                    note: err,
                },
                false,
            );
            self.start_pending_queued_job(ctx);
            return;
        }

        self.start_sync_entry(job_idx, entry.trigger, entry.retry_attempt, ctx);
    }

    pub(super) fn trigger_scheduled_sync_if_due(&mut self, ctx: &egui::Context) {
        let due_jobs = collect_due_scheduled_jobs_at(&self.config, Utc::now());
        if due_jobs.is_empty() {
            return;
        }

        let now = Utc::now();
        for idx in due_jobs {
            if self
                .session
                .as_ref()
                .map(|session| session.job_id == self.config.jobs[idx].id)
                .unwrap_or(false)
                || queue_contains_job(&self.job_queue, self.config.jobs[idx].id)
            {
                continue;
            }
            enqueue_job(&mut self.job_queue, &mut self.pending_queue_start, QueueEntry {
                job_id: self.config.jobs[idx].id,
                trigger: RunTrigger::Scheduled,
                retry_attempt: 0,
                ready_at: now,
            });
        }

        if self.job_queue.is_empty() {
            return;
        }

        self.save();
        self.start_pending_queued_job(ctx);
    }

    pub(super) fn request_schedule_wake_if_needed(&self, ctx: &egui::Context) {
        if has_enabled_schedule(&self.config) {
            ctx.request_repaint_after(std::time::Duration::from_secs(30));
        }
        if let Some(entry) = self.job_queue.front() {
            if entry.ready_at > Utc::now() {
                let wait = (entry.ready_at - Utc::now())
                    .to_std()
                    .unwrap_or_else(|_| std::time::Duration::from_secs(1));
                ctx.request_repaint_after(wait.min(std::time::Duration::from_secs(30)));
            }
        }
    }

    pub(super) fn enqueue_job(&mut self, entry: QueueEntry) {
        enqueue_job(&mut self.job_queue, &mut self.pending_queue_start, entry);
    }

    pub(super) fn requires_risk_confirmation(&self, idx: usize) -> bool {
        self.config.jobs.get(idx).map_or(false, |job| {
            job.sync_mode == SyncMode::Mirror
                || matches!(job.delete_mode, crate::model::job::DeleteMode::Direct)
        })
    }

    pub(super) fn find_job_idx_by_id(&self, job_id: uuid::Uuid) -> Option<usize> {
        self.config.jobs.iter().position(|job| job.id == job_id)
    }
}

fn queue_contains_job(
    queue: &std::collections::VecDeque<QueueEntry>,
    job_id: uuid::Uuid,
) -> bool {
    queue.iter().any(|entry| entry.job_id == job_id)
}

fn enqueue_job(
    queue: &mut std::collections::VecDeque<QueueEntry>,
    pending_queue_start: &mut bool,
    entry: QueueEntry,
) {
    if queue_contains_job(queue, entry.job_id) {
        return;
    }

    let insert_at = queue
        .iter()
        .position(|existing| existing.ready_at > entry.ready_at)
        .unwrap_or(queue.len());
    queue.insert(insert_at, entry);
    *pending_queue_start = true;
}

pub(super) fn has_enabled_schedule(config: &AppConfig) -> bool {
    config.jobs.iter().any(|j| {
        let runtime = config
            .job_states
            .iter()
            .find(|state| state.job_id == j.id)
            .map(|state| &state.schedule_runtime);
        j.schedule.enabled
            && j.schedule.interval_minutes > 0
            && !runtime.map(|state| state.paused).unwrap_or(false)
            && (j.sync_mode != SyncMode::Mirror || j.schedule.risk_acknowledged)
    })
}

pub(super) fn collect_due_scheduled_jobs_at(config: &AppConfig, now: DateTime<Utc>) -> Vec<usize> {
    let mut due = Vec::new();
    for (i, job) in config.jobs.iter().enumerate() {
        let state = config.job_states.iter().find(|state| state.job_id == job.id);
        if !job.schedule.enabled
            || job.schedule.interval_minutes == 0
            || state.map(|state| state.schedule_runtime.paused).unwrap_or(false)
            || (job.sync_mode == SyncMode::Mirror && !job.schedule.risk_acknowledged)
        {
            continue;
        }
        let last_sync_time = state.and_then(|state| state.last_sync_time);
        if is_schedule_due(last_sync_time, job.schedule.interval_minutes, now) {
            due.push(i);
        }
    }
    due.sort_by_key(|idx| {
        config
            .job_states
            .iter()
            .find(|state| state.job_id == config.jobs[*idx].id)
            .and_then(|state| state.last_sync_time)
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
    });
    due
}
