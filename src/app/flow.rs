use super::*;

impl FileSyncApp {
    pub(super) fn handle_sync_completed(
        &mut self,
        session: &mut SyncSession,
        stats: crate::model::session::SyncStats,
        usn_checkpoints: std::collections::HashMap<String, (u64, i64)>,
        was_stopped: bool,
    ) {
        let finished_at = Utc::now();
        let elapsed_secs = (finished_at - session.started_at).num_seconds().max(0) as u64;
        let summary = RunSummary {
            copied: stats.copied_files,
            skipped: stats.skipped_files,
            errors: stats.error_count,
            deleted: stats.deleted_files,
            bytes: stats.copied_bytes,
            elapsed_secs,
        };
        let finished_job_idx = self.find_job_idx_by_id(session.job_id);
        let finished_job_name = session.job_name.clone();

        session.stats = stats;
        session.stats.speed_bps = 0;
        session.status = completed_session_status(was_stopped);
        for w in &mut session.active_workers {
            *w = WorkerState::Idle;
        }

        let log_data = crate::log::SyncLogData {
            job_name: &finished_job_name,
            started_at: session.started_at,
            finished_at,
            stats: &session.stats,
            copied_log: &session.copied_log,
            deleted_log: &session.deleted_paths,
            orphan_log: &session.orphan_log,
            errors: &session.errors,
        };
        if let Err(e) = crate::log::write_sync_log(&log_data) {
            crate::log::app_log(
                &format!("write_sync_log failed: {}", e),
                crate::log::LogLevel::Error,
            );
            self.error_message = Some(format!(
                "Sync completed, but writing the log failed: {}",
                e
            ));
        }

        self.sync_running = false;
        self.stop_signal = None;
        self.pending_delete_fallbacks.clear();

        let history_result = if was_stopped {
            RunResultStatus::Stopped
        } else if summary.errors > 0 {
            RunResultStatus::Warning
        } else {
            RunResultStatus::Completed
        };

        if let Some(idx) = finished_job_idx {
            if let Some(job_id) = self.config.jobs.get(idx).map(|job| job.id) {
                if !was_stopped {
                    let state = self.ensure_job_state_mut(job_id);
                    state.last_sync_time = Some(finished_at);
                    state.last_run_summary = Some(summary.clone());
                    if !usn_checkpoints.is_empty() {
                        for (vol, (journal_id, next_usn)) in usn_checkpoints {
                            self.job_transient
                                .entry(job_id)
                                .or_default()
                                .last_sync_checkpoints
                                .insert(
                                    vol,
                                    crate::model::job::UsnCheckpoint { journal_id, next_usn },
                                );
                        }
                    }
                }
            }
        }

        if let Some(idx) = finished_job_idx {
            let mut history_note = if was_stopped {
                "Run stopped on user request.".to_string()
            } else {
                String::new()
            };

            if !was_stopped {
                if let Some(job) = self.config.jobs.get(idx) {
                    if summary.errors > 0
                        && job.schedule.enabled
                        && job.schedule.retry_on_failure
                        && session.retry_attempt < job.schedule.max_retries as u32
                    {
                        let next_attempt = session.retry_attempt + 1;
                        let ready_at = finished_at
                            + chrono::Duration::minutes(job.schedule.retry_delay_minutes as i64);
                        self.enqueue_job(QueueEntry {
                            job_id: job.id,
                            trigger: RunTrigger::Retry,
                            retry_attempt: next_attempt,
                            ready_at,
                        });
                        history_note = format!(
                            "Retry {} scheduled for {}.",
                            next_attempt,
                            ready_at.with_timezone(&chrono::Local).format("%m-%d %H:%M")
                        );
                    }
                }
            }

            if history_note.is_empty() && summary.errors > 0 {
                history_note =
                    "This run completed with errors. Review the error log below.".to_string();
            }

            self.record_run_history(
                idx,
                RunHistoryEntry {
                    started_at: session.started_at,
                    finished_at,
                    trigger: session.trigger,
                    result: history_result,
                    retry_attempt: session.retry_attempt,
                    summary: if was_stopped {
                        None
                    } else {
                        Some(summary.clone())
                    },
                    note: history_note.clone(),
                },
                false,
            );
            self.apply_run_outcome(idx, session.trigger, history_result, history_note.as_str());
        }

        if should_record_sync_completion(was_stopped) {
            play_completion_sound();
            if !self.job_queue.is_empty() {
                self.pending_queue_start = true;
            }

            self.notification = Some(build_completion_notification(
                &finished_job_name,
                summary.copied,
                summary.skipped,
                summary.errors,
                summary.deleted,
                is_zh(),
            ));
        }
    }

    pub(super) fn handle_start_failed(&mut self, session: &mut SyncSession, message: String) {
        session.status = SessionStatus::Failed;
        for w in &mut session.active_workers {
            *w = WorkerState::Idle;
        }
        self.sync_running = false;
        self.stop_signal = None;
        self.pending_delete_fallbacks.clear();
        self.pending_mass_delete_confirmation = None;
        if let Some(idx) = self.find_job_idx_by_id(session.job_id) {
            self.record_run_history(
                idx,
                RunHistoryEntry {
                    started_at: session.started_at,
                    finished_at: Utc::now(),
                    trigger: session.trigger,
                    result: RunResultStatus::Failed,
                    retry_attempt: session.retry_attempt,
                    summary: None,
                    note: message.clone(),
                },
                false,
            );
            self.apply_run_outcome(idx, session.trigger, RunResultStatus::Failed, &message);
        }
        self.error_message = Some(format!("Failed to start sync: {}", message));
    }

    pub(super) fn handle_disk_full(&mut self, session: &mut SyncSession) {
        session.status = SessionStatus::Failed;
        self.sync_running = false;
        self.stop_signal = None;
        self.pending_delete_fallbacks.clear();
        self.pending_mass_delete_confirmation = None;
        let note = "Disk full, sync stopped.".to_string();
        if let Some(idx) = self.find_job_idx_by_id(session.job_id) {
            self.record_run_history(
                idx,
                RunHistoryEntry {
                    started_at: session.started_at,
                    finished_at: Utc::now(),
                    trigger: session.trigger,
                    result: RunResultStatus::Failed,
                    retry_attempt: session.retry_attempt,
                    summary: None,
                    note: note.clone(),
                },
                false,
            );
            self.apply_run_outcome(idx, session.trigger, RunResultStatus::Failed, &note);
        }
        self.error_message = Some("Disk full, sync stopped.".to_string());
    }

    pub(super) fn refresh_speed(session: &mut SyncSession) {
        let now = std::time::Instant::now();
        let elapsed = now.saturating_duration_since(session.last_speed_sample_at);
        if elapsed < std::time::Duration::from_millis(250) {
            return;
        }

        let current_bytes = effective_copied_bytes(session);
        let delta_bytes = current_bytes.saturating_sub(session.last_speed_sample_bytes);
        let secs = elapsed.as_secs_f64();
        if secs > 0.0 {
            session.stats.speed_bps = (delta_bytes as f64 / secs) as u64;
        }
        session.last_speed_sample_at = now;
        session.last_speed_sample_bytes = current_bytes;
    }

    pub(super) fn record_run_history(
        &mut self,
        idx: usize,
        entry: RunHistoryEntry,
        update_last_summary: bool,
    ) {
        const MAX_HISTORY: usize = 20;
        let mut should_save = false;

        if let Some(job_id) = self.config.jobs.get(idx).map(|job| job.id) {
            let state = self.ensure_job_state_mut(job_id);
            if update_last_summary {
                if entry.result != RunResultStatus::Stopped
                    && entry.result != RunResultStatus::Missed
                {
                    state.last_sync_time = Some(entry.finished_at);
                }
                state.last_run_summary = entry.summary.clone();
            }
            state.run_history.insert(0, entry);
            if state.run_history.len() > MAX_HISTORY {
                state.run_history.truncate(MAX_HISTORY);
            }
            should_save = true;
        }

        if should_save {
            if let Err(e) = crate::config::storage::save(&self.config) {
                crate::log::app_log(
                    &format!("auto-save after history update failed: {}", e),
                    LogLevel::Error,
                );
            }
        }
    }

    pub(super) fn apply_run_outcome(
        &mut self,
        idx: usize,
        trigger: RunTrigger,
        result: RunResultStatus,
        note: &str,
    ) {
        let mut should_save = false;
        if let Some(job) = self.config.jobs.get(idx) {
            let pause_limit = job.schedule.pause_after_failures.max(1);
            let runtime = &mut self.ensure_job_state_mut(job.id).schedule_runtime;
            let is_failure = matches!(result, RunResultStatus::Failed | RunResultStatus::Warning);
            if is_failure {
                runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
                if matches!(trigger, RunTrigger::Scheduled | RunTrigger::Retry)
                    && runtime.consecutive_failures >= pause_limit
                {
                    runtime.paused = true;
                    runtime.pause_reason = if note.is_empty() {
                        format!(
                            "Scheduled sync paused after {} consecutive failures.",
                            runtime.consecutive_failures
                        )
                    } else {
                        format!(
                            "Scheduled sync paused after {} consecutive failures. {}",
                            runtime.consecutive_failures, note
                        )
                    };
                }
            } else if matches!(result, RunResultStatus::Completed) {
                runtime.consecutive_failures = 0;
                runtime.paused = false;
                runtime.pause_reason.clear();
            }
            should_save = true;
        }
        if should_save {
            if let Err(e) = crate::config::storage::save(&self.config) {
                crate::log::app_log(
                    &format!("auto-save after schedule outcome update failed: {}", e),
                    LogLevel::Error,
                );
            }
        }
    }
}
