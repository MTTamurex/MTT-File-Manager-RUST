//! Recycle bin operations: restore, delete permanently, empty
//!
//! This module handles operations specific to the Windows Recycle Bin.

use std::path::{Path, PathBuf};
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn restore_from_recycle_bin(&mut self, physical_path: &Path) {
        use crate::infrastructure::windows::recycle_bin::enumerate_recycle_bin;

        // Get the original path from RecycleBinItem by re-enumerating
        // This ensures we get the correct original_path stored in the $I file
        let original_path = if let Ok(recycle_items) = enumerate_recycle_bin() {
            recycle_items
                .iter()
                .find(|item| item.physical_path == physical_path)
                .map(|item| item.original_path.clone())
        } else {
            None
        };

        let item_name = self.items.iter()
            .find(|i| i.path == physical_path)
            .map(|i| i.name.clone());

        if let Some(name) = item_name {
            let original_path = original_path.unwrap_or_else(|| {
                // Fallback: use Desktop if we can't find original path
                PathBuf::from("C:\\Users\\Public\\Desktop").join(name.clone())
            });

            // If we are restoring the currently selected file, reset selection
            if let Some(selected) = &self.selected_file {
                if selected.path == physical_path {
                    self.reset_selection_and_search();
                }
            }

            // Send request to background worker
            let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::RestoreFromRecycleBin {
                physical_path: physical_path.to_path_buf(),
                original_path,
            });

            self.notifications
                .push(crate::application::AppNotification::info(format!(
                    "Restaurando '{}' em background...",
                    name
                )));
        }
    }

    pub fn delete_permanently(&mut self, physical_path: &Path) {
        let item_name = self.items.iter()
            .find(|i| i.path == physical_path)
            .map(|i| i.name.clone());

        if let Some(name) = item_name {
            // If we are deleting the currently selected file, reset selection
            if let Some(selected) = &self.selected_file {
                if selected.path == physical_path {
                    self.reset_selection_and_search();
                }
            }

            // Send request to background worker
            let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::DeletePermanently {
                physical_path: physical_path.to_path_buf(),
            });

            self.notifications
                .push(crate::application::AppNotification::info(format!(
                    "Excluindo '{}' permanentemente...",
                    name
                )));
        }
    }

    pub fn empty_recycle_bin(&mut self) {
        // Clear selection first so details panel resets immediately
        self.reset_selection_and_search();

        // Send request to background worker
        let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::EmptyRecycleBin);

        self.notifications
            .push(crate::application::AppNotification::info(
                "Esvaziando lixeira em background...".to_string(),
            ));
    }
}
