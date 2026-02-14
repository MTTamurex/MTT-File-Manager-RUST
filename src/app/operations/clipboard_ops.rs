//! Clipboard operations: copy, cut, paste, copy path
//!
//! This module handles interaction with the Windows clipboard for file operations.

use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    pub fn command_copy(&mut self, idx: Option<usize>) {
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

        // Destination folder
        let dest_folder = if let Some(idx) = idx {
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
            self.file_ops_in_progress += 1;
            let _ = self.file_op_sender.send(req);

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
