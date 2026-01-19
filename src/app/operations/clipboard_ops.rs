//! Clipboard operations: copy, cut, paste, copy path
//!
//! This module handles interaction with the Windows clipboard for file operations.

use std::path::{Path, PathBuf};
use crate::app::state::ImageViewerApp;
use crate::application::file_operations;

impl ImageViewerApp {
    pub fn command_copy(&mut self, idx: Option<usize>) {
        if let Some(idx) = idx.or(self.selected_item) {
            if let Some(item) = self.items.get(idx) {
                self.clipboard.copy(&item.path);
            }
        }
    }

    /// Recortar: Coloca o arquivo no clipboard do Windows com flag de MOVE
    pub fn command_cut(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                self.clipboard.cut(&item.path);
            }
        }
    }

    /// Colar: Lê do clipboard usando ClipboardManager via Background Worker
    pub fn command_paste(&mut self, idx: Option<usize>) {
        eprintln!("[DEBUG] command_paste called with idx: {:?}", idx);

        // Destination folder
        let dest_folder = if let Some(idx) = idx {
            if let Some(item) = self.items.get(idx) {
                if item.is_dir {
                    item.path.clone()
                } else {
                    PathBuf::from(&self.current_path)
                }
            } else {
                PathBuf::from(&self.current_path)
            }
        } else {
            PathBuf::from(&self.current_path)
        };

        // 1. Get files and operation from clipboard (Shell or Internal)
        let mut files_to_op = Vec::new();
        let mut is_move = false;

        if let Some(files) = crate::infrastructure::windows_clipboard::get_files_from_clipboard() {
            let op = crate::infrastructure::windows_clipboard::get_clipboard_operation();
            is_move = matches!(op, Some(crate::infrastructure::windows_clipboard::ClipboardFileOp::Move));
            files_to_op = files;
        } else if let Some(path) = self.clipboard.internal_state().0 {
            is_move = matches!(self.clipboard.internal_state().1, Some(crate::application::clipboard::ClipboardOp::Move));
            files_to_op = vec![path.clone()];
        }

        // 2. Dispatch to worker for EACH file
        for src_path in files_to_op {
            let req = if is_move {
                crate::workers::file_operation_worker::FileOperationRequest::file_move(
                    src_path,
                    dest_folder.clone(),
                    self.native_hwnd.unwrap_or_default(),
                )
            } else {
                crate::workers::file_operation_worker::FileOperationRequest::copy(
                    src_path,
                    dest_folder.clone(),
                    self.native_hwnd.unwrap_or_default(),
                )
            };
            let _ = self.file_op_sender.send(req);
        }

        // Clear internal state if it was a move (Shell does this for us for system clipboard)
        if is_move {
            self.clipboard.clear();
        }

        self.context_menu.target_path = None;
    }

    pub fn copy_path_to_clipboard(&self, path: &Path) {
        if let Err(e) = file_operations::copy_path_to_clipboard(path) {
            eprintln!("Erro clipboard: {}", e);
        }
    }
}
