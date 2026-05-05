use std::path::Path;

use egui::Context;
use flume::Sender;

use crate::engine::events::{DeleteFallbackChoice, SyncEvent};
use crate::model::job::RunTrigger;

pub trait SyncInteraction: Send + Sync {
    fn allows_prompts(&self) -> bool;
    fn confirm_mass_delete(&self, count: u64) -> bool;
    fn request_delete_fallback(&self, path: &Path, is_dir: bool, message: String) -> DeleteFallbackChoice;
}

pub struct ChannelSyncInteraction {
    trigger: RunTrigger,
    tx: Sender<SyncEvent>,
    ctx: Context,
}

impl ChannelSyncInteraction {
    pub fn new(trigger: RunTrigger, tx: Sender<SyncEvent>, ctx: Context) -> Self {
        Self { trigger, tx, ctx }
    }
}

impl SyncInteraction for ChannelSyncInteraction {
    fn allows_prompts(&self) -> bool {
        matches!(self.trigger, RunTrigger::Manual)
    }

    fn confirm_mass_delete(&self, count: u64) -> bool {
        let (confirm_tx, response_rx) = std::sync::mpsc::channel();
        let _ = self.tx.send(SyncEvent::MassDeleteConfirmationRequired {
            count,
            response: confirm_tx,
        });
        self.ctx.request_repaint();
        response_rx.recv().unwrap_or(false)
    }

    fn request_delete_fallback(&self, path: &Path, is_dir: bool, message: String) -> DeleteFallbackChoice {
        let (response_tx, response_rx) = std::sync::mpsc::channel();
        let _ = self.tx.send(SyncEvent::DeleteFallbackRequired {
            path: path.to_path_buf(),
            is_dir,
            message,
            response: response_tx,
        });
        self.ctx.request_repaint();
        response_rx.recv().unwrap_or(DeleteFallbackChoice::Skip)
    }
}
