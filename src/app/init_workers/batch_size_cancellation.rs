//! Always acknowledge an interrupted current-generation batch request.
use crate::app::folder_size_state::BatchSizeResult;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc::Sender, Arc};

pub(super) struct BatchSizeCancellation<'a> {
    pub acknowledged: bool,
    pub sender: &'a Sender<BatchSizeResult>,
    pub path: &'a PathBuf,
    pub cancel: &'a Arc<AtomicBool>,
    pub generation: &'a Arc<AtomicU64>,
    pub request_generation: u64,
    pub request_epoch: u64,
    pub ctx: &'a eframe::egui::Context,
}

impl Drop for BatchSizeCancellation<'_> {
    fn drop(&mut self) {
        if !self.acknowledged
            && self.cancel.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == self.request_generation
        {
            let _ = self.sender.send(BatchSizeResult {
                folder_path: self.path.clone(),
                summary: None,
                complete: true,
                cancelled: true,
                request_epoch: self.request_epoch,
                request_generation: self.request_generation,
            });
            self.ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_current_request_is_acknowledged_but_old_navigation_is_not() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let path = PathBuf::from(r"C:\Data\Folder");
        let cancel = Arc::new(AtomicBool::new(true));
        let generation = Arc::new(AtomicU64::new(2));
        let ctx = eframe::egui::Context::default();
        for request_generation in [1, 2] {
            drop(BatchSizeCancellation {
                acknowledged: false,
                sender: &sender,
                path: &path,
                cancel: &cancel,
                generation: &generation,
                request_generation,
                request_epoch: 3,
                ctx: &ctx,
            });
        }
        let result = receiver.try_recv().unwrap();
        assert_eq!(result.folder_path, path);
        assert_eq!(result.request_generation, 2);
        assert_eq!(result.request_epoch, 3);
        assert!(result.complete);
        assert!(result.cancelled);
        assert!(result.summary.is_none());
        assert!(receiver.try_recv().is_err());
        drop(BatchSizeCancellation {
            acknowledged: true,
            sender: &sender,
            path: &path,
            cancel: &cancel,
            generation: &generation,
            request_generation: 2,
            request_epoch: 3,
            ctx: &ctx,
        });
        assert!(receiver.try_recv().is_err());
    }
}
