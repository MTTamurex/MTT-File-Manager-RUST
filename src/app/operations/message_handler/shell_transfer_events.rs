use crate::app::file_operation_state::release_pending_clipboard_move;
use crate::app::state::ImageViewerApp;
use std::path::PathBuf;

impl ImageViewerApp {
    pub(super) fn handle_shell_transfer_returned(
        &mut self,
        parent_folders: Vec<PathBuf>,
        clipboard_paste_token: Option<u64>,
        dispatch_failed: bool,
        current_path_norm: &str,
    ) {
        // End only the local dispatch lock. Do not erase the clipboard, mutate
        // tags/covers as if files moved, or announce a completed transfer.
        if let Some(token) = clipboard_paste_token {
            release_pending_clipboard_move(
                &mut self.file_operation_state.pending_clipboard_move,
                token,
            );
        }
        self.file_operation_state.file_ops_in_progress = self
            .file_operation_state
            .file_ops_in_progress
            .saturating_sub(1);
        if self.file_operation_state.file_ops_in_progress == 0
            && self
                .file_operation_state
                .batch_rename_progress
                .as_ref()
                .is_some_and(|progress| progress.completed >= progress.total)
        {
            self.file_operation_state.batch_rename_progress = None;
        }
        self.maybe_clear_pending_deletions();

        // Events may already have arrived while the choice dialog was open.
        // Reconcile once now, then leave further updates to normal watchers.
        // Unlike FinishedNoRefresh, do not suppress watchers or steal focus
        // from the external transfer UI while its job may still be running.
        let folders = parent_folders.iter().collect::<Vec<_>>();
        self.reload_inactive_panel_if_matches(&folders);
        self.handle_parent_folder_updates(parent_folders, current_path_norm);
        if dispatch_failed {
            self.notifications
                .warning(rust_i18n::t!("operations.error_cancelled").to_string());
        }
        self.ui_ctx.request_repaint();
    }
}
