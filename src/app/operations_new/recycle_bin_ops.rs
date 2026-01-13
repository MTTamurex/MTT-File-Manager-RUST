//! Recycle bin operations: restore, delete permanently, empty
//!
//! This module handles operations specific to the Windows Recycle Bin.

use std::path::{Path, PathBuf};
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn restore_from_recycle_bin(&mut self, physical_path: &Path) {
        use crate::infrastructure::windows::recycle_bin::{
            enumerate_recycle_bin, restore_from_recycle_bin,
        };

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

            match restore_from_recycle_bin(physical_path, &original_path) {
                Ok(_) => {
                    self.notifications
                        .push(crate::application::AppNotification::success(format!(
                            "'{}' restaurado com sucesso",
                            item.name
                        )));
                    // Refresh recycle bin view
                    self.setup_recycle_bin_view();
                }
                Err(e) => {
                    self.notifications
                        .push(crate::application::AppNotification::error(format!(
                            "Erro ao restaurar: {}",
                            e
                        )));
                }
            }
        }
    }

    pub fn delete_permanently(&mut self, physical_path: &Path) {
        use crate::infrastructure::windows::recycle_bin::delete_permanently;

        if let Some(item) = self.items.iter().find(|i| i.path == physical_path) {
            let item_name = item.name.clone();

            match delete_permanently(physical_path) {
                Ok(_) => {
                    self.notifications
                        .push(crate::application::AppNotification::success(format!(
                            "'{}' excluído permanentemente",
                            item_name
                        )));
                    // Refresh recycle bin view
                    self.setup_recycle_bin_view();
                }
                Err(e) => {
                    self.notifications
                        .push(crate::application::AppNotification::error(format!(
                            "Erro ao excluir: {}",
                            e
                        )));
                }
            }
        }
    }

    pub fn empty_recycle_bin(&mut self) {
        use crate::infrastructure::windows::recycle_bin::empty_recycle_bin;

        match empty_recycle_bin() {
            Ok(_) => {
                self.notifications
                    .push(crate::application::AppNotification::success(
                        "Lixeira esvaziada com sucesso".to_string(),
                    ));
                // Refresh recycle bin view
                self.setup_recycle_bin_view();
            }
            Err(e) => {
                self.notifications
                    .push(crate::application::AppNotification::error(format!(
                        "Erro ao esvaziar lixeira: {}",
                        e
                    )));
            }
        }
    }
}
