//! Clipboard operations: copy, cut, paste, copy path
//!
//! This module handles interaction with the Windows clipboard for file operations.

use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    pub fn can_copy_from_current_location(&self) -> bool {
        !self.navigation_state.is_recycle_bin_view
    }

    pub fn can_paste_into_current_location(&self) -> bool {
        self.clipboard.has_content()
            && !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
    }

    pub fn command_copy(&mut self, idx: Option<usize>) {
        if !self.can_copy_from_current_location() {
            self.context_menu.target_paths.clear();
            return;
        }

        if idx.is_none() && !self.context_menu.target_paths.is_empty() {
            self.clipboard
                .copy(&self.context_menu.target_paths.clone());
            return;
        }

        let mut files = Vec::new();

        let use_multi_selection = if let Some(i) = idx {
            if let Some(item) = self.items.get(i) {
                self.multi_selection.contains(&item.path)
            } else {
                false
            }
        } else {
            !self.multi_selection.is_empty()
        };

        if use_multi_selection {
            files.extend(self.multi_selection.iter().cloned());
        } else if let Some(i) = idx.or(self.selected_item) {
            if let Some(item) = self.items.get(i) {
                files.push(item.path.clone());
            }
        }

        if !files.is_empty() {
            self.clipboard.copy(&files);
        }
    }

    /// Recortar: Coloca o arquivo no clipboard do Windows com flag de MOVE
    pub fn command_cut(&mut self, idx: Option<usize>) {
        if idx.is_none() && !self.context_menu.target_paths.is_empty() {
            self.clipboard.cut(&self.context_menu.target_paths.clone());
            return;
        }

        let mut files = Vec::new();

        let use_multi_selection = if let Some(i) = idx {
            if let Some(item) = self.items.get(i) {
                self.multi_selection.contains(&item.path)
            } else {
                false
            }
        } else {
            !self.multi_selection.is_empty()
        };

        if use_multi_selection {
            files.extend(self.multi_selection.iter().cloned());
        } else if let Some(i) = idx.or(self.selected_item) {
            if let Some(item) = self.items.get(i) {
                files.push(item.path.clone());
            }
        }

        if !files.is_empty() {
            self.clipboard.cut(&files);
        }
    }

    /// Paste: Reads from clipboard using ClipboardManager via Background Worker
    pub fn command_paste(&mut self, idx: Option<usize>) {
        log::debug!("[DEBUG] command_paste called with idx: {:?}", idx);

        if !self.can_paste_into_current_location() {
            self.context_menu.target_paths.clear();
            return;
        }

        // Destination folder
        let dest_folder = if idx.is_none() && !self.context_menu.target_paths.is_empty() {
            self.context_menu
                .target_paths
                .first()
                .filter(|path| path.is_dir())
                .cloned()
                .unwrap_or_else(|| PathBuf::from(&self.navigation_state.current_path))
        } else if let Some(idx) = idx {
            if let Some(item) = self.items.get(idx) {
                if item.is_dir {
                    item.path.clone()
                } else {
                    PathBuf::from(&self.navigation_state.current_path)
                }
            } else {
                PathBuf::from(&self.navigation_state.current_path)
            }
        } else {
            PathBuf::from(&self.navigation_state.current_path)
        };

        // 1. Get files and operation from clipboard via Manager
        // Optimized to use the manager's logic which checks system then internal.
        if let Some((files_to_op, is_move)) = self.clipboard.get_files_to_paste() {
            let hwnd = self.native_hwnd.unwrap_or_default();

            // 2. Dispatch as a single batch operation (single Windows progress dialog)
            let req = if is_move {
                crate::workers::file_operation_worker::FileOperationRequest::move_batch(
                    files_to_op,
                    dest_folder,
                    hwnd,
                )
            } else {
                crate::workers::file_operation_worker::FileOperationRequest::copy_batch(
                    files_to_op,
                    dest_folder,
                    hwnd,
                )
            };
            self.file_operation_state.file_ops_in_progress += 1;
            if self.file_operation_state.file_op_sender.send(req).is_err() {
                self.file_operation_state.file_ops_in_progress =
                    self.file_operation_state.file_ops_in_progress.saturating_sub(1);
                log::warn!("[FileOps] H-3: worker channel closed on clipboard op");
            }

            // Clear internal state if it was a move (Shell does this for us for system clipboard)
            if is_move {
                self.clipboard.clear();
            }
        }

        self.context_menu.target_paths.clear();
    }

    pub fn copy_path_to_clipboard(&self, path: &Path) {
        if let Err(e) = file_operations::copy_path_to_clipboard(path) {
            log::error!("Erro clipboard: {}", e);
        }
    }
}
