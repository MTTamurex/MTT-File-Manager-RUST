//! Recycle bin operations: restore, delete permanently, empty
//!
//! This module handles operations specific to the Windows Recycle Bin.

use crate::app::state::ImageViewerApp;
use std::path::PathBuf;

impl ImageViewerApp {
    pub fn restore_from_recycle_bin(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let mut restore_items = Vec::with_capacity(paths.len());

        for physical_path in paths {
            // Check if we have the original path cached in our items list
            // Normalize path for lookup to be safe
            let entry = self.items.iter().find(|i| {
                // Precise match first
                if i.path == *physical_path {
                    return true;
                }

                // Fallback to normalized comparison if needed
                let p1 = i.path.to_string_lossy().to_lowercase();
                let p1 = p1.strip_prefix("\\\\?\\").unwrap_or(&p1);
                let p2 = physical_path.to_string_lossy().to_lowercase();
                let p2 = p2.strip_prefix("\\\\?\\").unwrap_or(&p2);
                p1 == p2
            });

            if let Some(item) = entry {
                let original = match item.recycle_original_path() {
                    Some(p) => p.to_path_buf(),
                    None => {
                        // Skip items with unknown original path rather than guessing
                        // a destination that could expose files to a public directory.
                        log::warn!(
                            "[RecycleBin] Skipping '{}': original path unknown, cannot restore safely",
                            item.name
                        );
                        continue;
                    }
                };

                restore_items.push((physical_path.clone(), original));
            } else {
                // Item not in self.items and no original path — skip rather than
                // guessing a destination that could expose files publicly.
                log::warn!(
                    "[RecycleBin] Skipping '{}': item not found, cannot determine original path",
                    physical_path.display()
                );
            }
        }

        // Send SINGLE batch request to worker
        if !restore_items.is_empty() {
            self.notifications
                .push(crate::application::AppNotification::info(format!(
                    "Restaurando {} itens...",
                    restore_items.len()
                )));
            self.file_operation_state.file_ops_in_progress += 1;
            if self.file_operation_state.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::RestoreFromRecycleBin {
                items: restore_items,
            }).is_err() {
                self.file_operation_state.file_ops_in_progress =
                    self.file_operation_state.file_ops_in_progress.saturating_sub(1);
                log::warn!("[FileOps] H-3: worker channel closed on restore");
            }

            // Clear selection after restore batch is sent
            self.reset_selection_and_search();
        }
    }

    pub fn delete_permanently(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        for physical_path in paths {
            // If we are deleting the currently selected file, reset selection
            if let Some(selected) = &self.selected_file {
                if selected.path == *physical_path {
                    self.reset_selection_and_search();
                }
            }
        }

        // Send request to background worker (BATCH)
        // Windows will show a native confirmation dialog before deleting.
        let hwnd = self.shell_op_hwnd();
        self.file_operation_state.file_ops_in_progress += 1;
        if self
            .file_operation_state
            .file_op_sender
            .send(
                crate::workers::file_operation_worker::FileOperationRequest::DeletePermanently {
                    physical_paths: paths.to_vec(),
                    hwnd: crate::workers::file_operation_worker::SendHwnd(hwnd),
                },
            )
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[FileOps] H-3: worker channel closed on DeletePermanently");
        }

        // Clear selection after delete batch is sent
        self.reset_selection_and_search();
    }

    pub fn empty_recycle_bin(&mut self) {
        // Clear selection first so details panel resets immediately
        self.reset_selection_and_search();

        // Send request to background worker.
        // Windows will show a native confirmation dialog before emptying.
        let hwnd = self.shell_op_hwnd();
        self.file_operation_state.file_ops_in_progress += 1;
        if self
            .file_operation_state
            .file_op_sender
            .send(
                crate::workers::file_operation_worker::FileOperationRequest::EmptyRecycleBin {
                    hwnd: crate::workers::file_operation_worker::SendHwnd(hwnd),
                },
            )
            .is_err()
        {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[FileOps] H-3: worker channel closed on EmptyRecycleBin");
        }
    }
}
