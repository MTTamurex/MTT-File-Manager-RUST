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

    /// Colar: Lê do clipboard usando ClipboardManager
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

        match self.clipboard.paste(&dest_folder, self.native_hwnd) {
            Ok(true) => {
                // Move successful
                self.load_folder(false);
                self.context_menu.target_path = None;
            }
            Ok(false) => {
                // Copy successful
                self.load_folder(false);
                self.context_menu.target_path = None;
            }
            Err(e) => {
                eprintln!("[CLIPBOARD ERROR] {}", e);
            }
        }
    }

    pub fn copy_path_to_clipboard(&self, path: &Path) {
        if let Err(e) = file_operations::copy_path_to_clipboard(path) {
            eprintln!("Erro clipboard: {}", e);
        }
    }
}
