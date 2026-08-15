use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct BatchRenameProgress {
    pub completed: usize,
    pub total: usize,
    pub current_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PendingClipboardMove {
    pub(crate) token: u64,
    pub(crate) clipboard_sequence: Option<u32>,
}

pub(crate) fn pending_clipboard_move_after_dispatch(
    current: Option<PendingClipboardMove>,
    dispatched: PendingClipboardMove,
    send_succeeded: bool,
) -> Option<PendingClipboardMove> {
    if send_succeeded && current.is_none() {
        Some(dispatched)
    } else {
        current
    }
}

pub(crate) fn complete_pending_clipboard_move(
    pending: &mut Option<PendingClipboardMove>,
    completed_token: Option<u64>,
    current_sequence: Option<u32>,
) -> bool {
    let Some(completed_token) = completed_token else {
        return false;
    };
    let Some(current) = *pending else {
        return false;
    };
    if current.token != completed_token {
        return false;
    }

    *pending = None;
    current.clipboard_sequence.is_some() && current.clipboard_sequence == current_sequence
}

pub(crate) fn fail_pending_clipboard_move(
    pending: &mut Option<PendingClipboardMove>,
    failed_token: u64,
) -> bool {
    if pending.is_some_and(|current| current.token == failed_token) {
        *pending = None;
        true
    } else {
        false
    }
}

pub struct FileOperationState {
    pub(crate) file_op_sender: Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    pub file_op_res_receiver: Receiver<crate::workers::file_operation_worker::FileOperationResult>,
    pub deferred_results: VecDeque<crate::workers::file_operation_worker::FileOperationResult>,
    pub extraction_progress: crate::infrastructure::archive_extract::SharedExtractionProgress,
    pub extraction_cancel: crate::infrastructure::archive_extract::ExtractionCancelFlag,
    pub(crate) compression_sender:
        Sender<crate::workers::archive_compression_worker::ArchiveCompressionRequest>,
    pub compression_progress: crate::infrastructure::archive_create::SharedCompressionProgress,
    pub compression_cancel: crate::infrastructure::archive_create::CompressionCancelFlag,
    pub disk_cache_invalidation_sender:
        Sender<Vec<crate::app::init_workers::CacheInvalidationEntry>>,
    pub prefetch_sender: Sender<crate::workers::prefetch_worker::PrefetchMessage>,
    pub idle_warmup_sender: Sender<crate::workers::idle_warmup::IdleWarmupMessage>,
    pub file_ops_in_progress: usize,
    pub batch_rename_progress: Option<BatchRenameProgress>,
    pub(crate) pending_clipboard_move: Option<PendingClipboardMove>,
    pub(crate) pending_clipboard_cleanup_sequence: Option<u32>,
    pub(crate) next_clipboard_move_token: u64,
    pub pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,
    pub pending_iso_mount: Option<PathBuf>,
    pub mounted_iso_drives: HashMap<String, PathBuf>,
    /// One-shot receiver for pre-mounted ISO detection at startup.
    pub iso_detect_rx: Option<Receiver<HashMap<String, PathBuf>>>,
}

impl FileOperationState {
    pub(crate) fn allocate_clipboard_move_token(&mut self) -> u64 {
        self.next_clipboard_move_token = self.next_clipboard_move_token.wrapping_add(1);
        self.next_clipboard_move_token
    }
}

#[cfg(test)]
mod tests {
    use super::{
        complete_pending_clipboard_move, fail_pending_clipboard_move,
        pending_clipboard_move_after_dispatch, PendingClipboardMove,
    };

    fn pending(token: u64, sequence: u32) -> PendingClipboardMove {
        PendingClipboardMove {
            token,
            clipboard_sequence: Some(sequence),
        }
    }

    #[test]
    fn failed_send_does_not_register_clipboard_move() {
        assert_eq!(
            pending_clipboard_move_after_dispatch(None, pending(1, 10), false),
            None
        );
    }

    #[test]
    fn matching_success_with_unchanged_sequence_clears() {
        let mut current = Some(pending(2, 20));

        assert!(complete_pending_clipboard_move(
            &mut current,
            Some(2),
            Some(20)
        ));
        assert_eq!(current, None);
    }

    #[test]
    fn changed_sequence_preserves_clipboard() {
        let mut current = Some(pending(3, 30));

        assert!(!complete_pending_clipboard_move(
            &mut current,
            Some(3),
            Some(31)
        ));
        assert_eq!(current, None);
    }

    #[test]
    fn different_token_cannot_complete_newer_clipboard_move() {
        let expected = pending(5, 50);
        let mut current = Some(expected);

        assert!(!complete_pending_clipboard_move(
            &mut current,
            Some(4),
            Some(50)
        ));
        assert_eq!(current, Some(expected));
    }

    #[test]
    fn move_not_originated_from_clipboard_never_clears() {
        let expected = pending(6, 60);
        let mut current = Some(expected);

        assert!(!complete_pending_clipboard_move(
            &mut current,
            None,
            Some(60)
        ));
        assert_eq!(current, Some(expected));
    }

    #[test]
    fn matching_failure_releases_pending_move_without_clearing_clipboard() {
        let mut current = Some(pending(7, 70));
        assert!(fail_pending_clipboard_move(&mut current, 7));
        assert_eq!(current, None);
    }

    #[test]
    fn second_dispatch_does_not_replace_pending_move() {
        let current = pending(8, 80);
        assert_eq!(
            pending_clipboard_move_after_dispatch(Some(current), pending(9, 90), true),
            Some(current)
        );
    }
}
