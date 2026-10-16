//! File transfers between the active and inactive panels.

use crate::app::operations::file_operation_policy::path_accepts_file_children;
use crate::app::state::ImageViewerApp;
use std::path::PathBuf;

impl ImageViewerApp {
    /// Copy selected items from the active panel to the inactive panel's folder.
    pub fn dual_panel_copy_to_other(&mut self) {
        if !self.dual_panel_enabled {
            return;
        }
        let Some(dest_path) = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|s| s.path.clone())
        else {
            return;
        };
        let dest = PathBuf::from(&dest_path);
        if !path_accepts_file_children(&dest) {
            return;
        }

        let sources = self.gather_selected_paths();
        if sources.is_empty()
            || sources
                .iter()
                .any(|path| self.path_is_network_operation_root(path))
        {
            return;
        }

        log::info!(
            "[DualPanel] Copy {} items to other panel: {:?}",
            sources.len(),
            dest
        );
        let hwnd = self.shell_op_hwnd();
        let req = crate::workers::file_operation_worker::FileOperationRequest::copy_batch(
            sources, dest, hwnd,
        );
        self.file_operation_state.file_ops_in_progress += 1;
        if self.file_operation_state.file_op_sender.send(req).is_err() {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[DualPanel] worker channel closed on cross-panel copy");
        }
    }

    /// Move selected items from the active panel to the inactive panel's folder.
    pub fn dual_panel_move_to_other(&mut self) {
        if !self.dual_panel_enabled {
            return;
        }
        let Some(dest_path) = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|s| s.path.clone())
        else {
            return;
        };
        let dest = PathBuf::from(&dest_path);
        if !path_accepts_file_children(&dest) {
            return;
        }

        let sources = self.gather_selected_paths();
        if sources.is_empty()
            || sources
                .iter()
                .any(|path| self.path_is_network_operation_root(path))
        {
            return;
        }

        log::info!(
            "[DualPanel] Move {} items to other panel: {:?}",
            sources.len(),
            dest
        );
        let hwnd = self.shell_op_hwnd();
        let req = crate::workers::file_operation_worker::FileOperationRequest::move_batch(
            sources, dest, hwnd,
        );
        self.file_operation_state.file_ops_in_progress += 1;
        if self.file_operation_state.file_op_sender.send(req).is_err() {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[DualPanel] worker channel closed on cross-panel move");
        }
    }

    /// Gather selected file paths from the active panel (multi-selection or single).
    fn gather_selected_paths(&self) -> Vec<PathBuf> {
        if !self.multi_selection.is_empty() {
            self.multi_selection.iter().cloned().collect()
        } else if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                vec![item.path.clone()]
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }
}
