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

        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let original_path = original_path.unwrap_or_else(|| {
                // Fallback: use Desktop if we can't find original path
                PathBuf::from("C:\\Users\\Public\\Desktop").join(item.name.clone())
            });

            // Send request to background worker
            let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::RestoreFromRecycleBin {
                physical_path: physical_path.to_path_buf(),
                original_path,
            });

            self.notifications
                .push(crate::application::AppNotification::info(format!(
                    "Restaurando '{}' em background...",
                    item.name
                )));
        }
    }

    pub fn delete_permanently(&mut self, physical_path: &Path) {

        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let item_name = item.name.clone();

            // Send request to background worker
            let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::DeletePermanently {
                physical_path: physical_path.to_path_buf(),
            });

            self.notifications
                .push(crate::application::AppNotification::info(format!(
                    "Excluindo '{}' permanentemente...",
                    item_name
                )));
        }
    }

    pub fn empty_recycle_bin(&mut self) {

        // Send request to background worker
        let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::EmptyRecycleBin);

        self.notifications
            .push(crate::application::AppNotification::info(
                "Esvaziando lixeira em background...".to_string(),
            ));
    }
}
