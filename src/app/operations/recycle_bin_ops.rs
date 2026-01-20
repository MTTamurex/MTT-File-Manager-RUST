//! Recycle bin operations: restore, delete permanently, empty
//!
//! This module handles operations specific to the Windows Recycle Bin.

use std::path::{Path, PathBuf};
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn restore_from_recycle_bin(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() { return; }

        let mut restore_items = Vec::with_capacity(paths.len());
        
        for physical_path in paths {
            // Check if we have the original path cached in our items list
            // Normalize path for lookup to be safe
            let entry = self.items.iter().find(|i| {
                // Precise match first
                if i.path == *physical_path { return true; }
                
                // Fallback to normalized comparison if needed
                let p1 = i.path.to_string_lossy().to_lowercase();
                let p1 = p1.strip_prefix("\\\\?\\").unwrap_or(&p1);
                let p2 = physical_path.to_string_lossy().to_lowercase();
                let p2 = p2.strip_prefix("\\\\?\\").unwrap_or(&p2);
                p1 == p2
            });

            if let Some(item) = entry {
                let original = item.recycle_original_path.clone().unwrap_or_else(|| {
                    // Critical fallback: if missing original path, try to guess from physical filename
                    PathBuf::from("C:\\Users\\Public\\Desktop").join(&item.name)
                });
                
                restore_items.push((physical_path.clone(), original));
            } else {
                // Handle case where item is not in self.items (should be rare)
                let name = physical_path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Item".to_string());
                    
                let original = PathBuf::from("C:\\Users\\Public\\Desktop").join(&name);
                restore_items.push((physical_path.clone(), original));
            }
        }

        // Send SINGLE batch request to worker
        if !restore_items.is_empty() {
            self.notifications.push(crate::application::AppNotification::info(format!(
                "Restaurando {} itens...",
                restore_items.len()
            )));
            let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::RestoreFromRecycleBin {
                items: restore_items,
            });

            // Clear selection after restore batch is sent
            self.reset_selection_and_search();
        }
    }

    pub fn delete_permanently(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() { return; }

        for physical_path in paths {
            let item_name = self.items.iter()
                .find(|i| i.path == *physical_path)
                .map(|i| i.name.clone())
                .unwrap_or_else(|| physical_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());

            // If we are deleting the currently selected file, reset selection
            if let Some(selected) = &self.selected_file {
                if selected.path == *physical_path {
                    self.reset_selection_and_search();
                }
            }
        }

        // Send request to background worker (BATCH)
        if !paths.is_empty() {
             self.notifications.push(crate::application::AppNotification::info(format!(
                "Excluindo {} itens permanentemente...",
                paths.len()
            )));
            let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::DeletePermanently {
                physical_paths: paths.to_vec(),
            });

            // Clear selection after delete batch is sent
            self.reset_selection_and_search();
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
