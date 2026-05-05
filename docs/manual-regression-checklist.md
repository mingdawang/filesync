# Manual Regression Checklist

This checklist covers the highest-risk paths after `src/app.rs` was split into submodules.

## Core Sync Flow

- Start a manual sync from the selected job.
- Stop an in-progress sync and confirm the UI returns to idle.
- Run a sync that completes without errors and confirm history is written.
- Run a sync that hits an error and confirm the warning path is visible in history.

## Queue and Schedule

- Queue two jobs with different `ready_at` times and confirm the earlier one starts first.
- Confirm duplicate queued jobs are not inserted twice.
- Trigger a scheduled job and confirm it starts automatically when due.
- Trigger repeated scheduled failures and confirm the job pauses after the configured threshold.
- Run a successful sync after a pause and confirm the failure counter resets.

## Delete Safety

- Run a mirror sync with a large delete count and confirm the mass-delete confirmation appears.
- Force a recycle-bin fallback and confirm the delete fallback dialog appears.
- Choose `Delete Directly`, `Skip`, and `Stop Sync` in separate runs and verify behavior.

## Window and Dialog Flow

- Close the app with no unsaved changes and verify configured close behavior.
- Close the app with unsaved changes and verify `Save`, `Don't Save`, and `Cancel`.
- Open and close Settings, About, and Task History.

## Preview and Notification

- Run preview scan for a valid job.
- Run preview scan with a missing source path and confirm the error text is readable.
- Complete a sync and confirm the completion notification appears and auto-dismisses.

## Localization Smoke Check

- On a Chinese UI environment, verify the split modules use translated labels instead of fallback English for the main dialog and settings surfaces.
- On an English UI environment, verify the same paths remain readable and consistent.
