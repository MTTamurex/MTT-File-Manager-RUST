//! File operations: delete, create folder, rename, properties, shortcuts
//!
//! This module handles basic file operations interacting with the shell.

use std::path::{Path, PathBuf};
use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use crate::domain::file_entry::FileEntry;

impl ImageViewerApp {
    pub fn delete_with_shell_for_idx(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                // Send request to background worker
                let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::delete(
                    item.path.clone(),
                    self.native_hwnd.unwrap_or_default(),
                ));

                // Clear cache and selection proactively (the worker will do the actual delete)
                self.disk_cache.remove_cache_for_path(&item.path);
                if self.selected_item == Some(idx) {
                    self.selected_item = None;
                    self.selected_file = None;
                }
            }
        }
    }

    pub fn show_properties_for_idx(&mut self, idx: Option<usize>) {
        let target_idx = idx.or(self.selected_item);
        if let Some(idx) = target_idx {
            if let Some(item) = self.items.get(idx) {
                let path = item.path.clone();
                // We'll use the shell properties dialog
                let _ = crate::infrastructure::windows::native_menu::show_properties_dialog(
                    self.native_hwnd.unwrap_or_default(),
                    &path,
                );
            }
        }
    }

    pub fn create_new_folder(&mut self) {
        let base_path = PathBuf::from(&self.current_path);

        match file_operations::create_new_folder(&base_path) {
            Ok(full_path) => {
                let new_folder_name = full_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                // CRITICAL: Immediately create entry to allow renaming
                let new_item = FileEntry::from_path(full_path.clone(), true);

                self.all_items.push(new_item);
                self.filter_items();
                self.sort_items();

                // Find index in filtered vector
                if let Some(idx) = self.items.iter().position(|i| i.path == full_path) {
                    self.selected_item = Some(idx);
                    self.selected_file = Some(self.items[idx].clone());
                    self.renaming_state = Some((idx, new_folder_name));
                    self.focus_rename = true;
                }

                // Request background real load to sync with disk
                self.load_folder(false);
            }
            Err(e) => {
                eprintln!("Erro ao criar pasta: {}", e);
            }
        }
    }

    /// Renomeia arquivo usando Shell API via Background Worker
    pub fn rename_with_shell(&mut self, idx: usize) {
        if let Some((_, new_name)) = self.renaming_state.take() {
            if let Some(item) = self.items.get(idx) {
                // Send request to background worker
                let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::rename(
                    item.path.clone(),
                    new_name,
                    self.native_hwnd.unwrap_or_default(),
                ));
            }
        }
    }

    /// Create a Windows shell shortcut (.lnk) pointing to `target` in the same directory.
    pub fn create_shell_shortcut(&self, target: &Path) -> Result<PathBuf, String> {
        file_operations::create_shortcut(target, &self.current_path)
    }
}
