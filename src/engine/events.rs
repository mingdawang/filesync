use std::collections::HashMap;
use std::path::PathBuf;

use crate::model::session::{ErrorScope, SyncStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteFallbackChoice {
    DirectDelete,
    Skip,
    StopSync,
}

/// Events emitted by the sync engine and consumed by the UI thread.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SyncEvent {
    /// Planning finished and execution is about to start.
    Started {
        total_files: u64,
        total_bytes: u64,
    },
    /// A worker started copying a file.
    FileStarted {
        worker_id: usize,
        path: PathBuf,
        size: u64,
        /// `true` for a brand-new file, `false` for an overwrite/update.
        is_new: bool,
    },
    /// Incremental copy progress for an active file.
    FileProgress {
        worker_id: usize,
        bytes_done: u64,
    },
    /// A worker started deleting a file or directory.
    DeleteStarted {
        worker_id: usize,
        path: PathBuf,
        is_dir: bool,
    },
    /// A file finished copying.
    FileCompleted {
        worker_id: usize,
        path: PathBuf,
        size: u64,
        /// Whether delta transfer was used.
        delta: bool,
        /// Bytes saved by delta transfer, or `0` for a full copy.
        saved_bytes: u64,
    },
    /// A file was skipped because no work was required.
    FileSkipped {
        path: PathBuf,
    },
    /// Recycle Bin fallback confirmation is required.
    DeleteFallbackRequired {
        path: PathBuf,
        is_dir: bool,
        message: String,
        response: std::sync::mpsc::Sender<DeleteFallbackChoice>,
    },
    /// Mirror delete volume crossed the safety threshold and requires confirmation.
    MassDeleteConfirmationRequired {
        count: u64,
        response: std::sync::mpsc::Sender<bool>,
    },
    /// A file-level error occurred without aborting the entire sync.
    FileError {
        path: PathBuf,
        message: String,
        scope: ErrorScope,
    },
    /// Mirror mode deleted an orphan destination item.
    FileDeleted {
        worker_id: usize,
        path: PathBuf,
    },
    WorkerFinished {
        worker_id: usize,
    },
    /// Update mode detected an orphan destination item and reported it only.
    FileOrphan {
        path: PathBuf,
    },
    /// Periodic transfer speed update in bytes per second.
    SpeedUpdate {
        bps: u64,
    },
    Paused,
    Resumed,
    /// The destination ran out of disk space.
    DiskFull,
    /// The sync task failed to start, for example if runtime creation failed.
    StartFailed {
        message: String,
    },
    /// All processing finished.
    Completed {
        stats: SyncStats,
        /// Fresh USN checkpoints keyed by volume root.
        usn_checkpoints: HashMap<String, (u64, i64)>,
        /// `true` when the user explicitly stopped this run.
        was_stopped: bool,
    },
}
