//! Operations on explicit search selections, independent of the folder behind the overlay.
use crate::app::state::ImageViewerApp;
use crate::workers::file_operation_worker::FileOperationRequest;
use std::path::PathBuf;

impl ImageViewerApp {
    pub(crate) fn delete_global_search_paths(&mut self, paths: &[PathBuf], permanently: bool) {
        if !self.can_delete_with_shell_paths(paths) {
            self.notifications
                .warning(rust_i18n::t!("operations.selection_operation_blocked").to_string());
            return;
        }
        let request =
            FileOperationRequest::search_delete(paths.to_vec(), permanently, self.shell_op_hwnd());
        self.file_operation_state.file_ops_in_progress += 1;
        if self
            .file_operation_state
            .file_op_sender
            .send(request)
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            self.notifications
                .warning(rust_i18n::t!("operations.error_worker_unavailable").to_string());
        }
    }
}
