//! File operations: delete, create folder, rename, properties, shortcuts
//!
//! This module handles basic file operations interacting with the shell.

use std::path::{Path, PathBuf};
use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use crate::domain::file_entry::FileEntry;

impl ImageViewerApp {
    pub fn delete_with_shell_for_idx(&mut self, idx: Option<usize>) {
        let paths = self.context_target_paths(idx);
        self.delete_with_shell_for_paths(&paths);
    }

    pub fn delete_with_shell_for_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() { return; }

        // Send request to background worker (BATCH)
        self.file_ops_in_progress += 1;
        let _ = self.file_op_sender.send(crate::workers::file_operation_worker::FileOperationRequest::delete(
            paths.to_vec(),
            self.native_hwnd.unwrap_or_default(),
        ));

        for path in paths {
            // Clear cache and selection proactively
            self.disk_cache.remove_cache_for_path(path);
            self.multi_selection.remove(path);
        }

        // Reset primary selection if it was deleted
        if let Some(selected) = &self.selected_file {
             if paths.contains(&selected.path) {
                 self.selected_item = None;
                 self.selected_file = None;
             }
        }
    }

    pub fn show_properties_for_idx(&mut self, idx: Option<usize>) {
        let paths = self.context_target_paths(idx);
        if paths.is_empty() { return; }

        if let Some(hwnd) = self.native_hwnd {
            // Use shell context menu to invoke properties (handles single and multiple files)
            if let Ok(shell_ctx) = crate::infrastructure::windows::native_menu::extract_shell_menu(hwnd, &paths) {
                 let items = shell_ctx.items.borrow();

                 // Look for properties verb
                 let mut prop_id = None;
                 for item in items.iter() {
                     if let Some(verb) = &item.command_string {
                         if verb.eq_ignore_ascii_case("properties") {
                             prop_id = Some(item.id);
                             break;
                         }
                     }
                 }

                 if let Some(id) = prop_id {
                     let _ = crate::infrastructure::windows::native_menu::invoke_menu_command(
                        hwnd,
                        &shell_ctx.context_menu,
                        id,
                        0, 0
                     );
                     return;
                 }
            }

            // Fallback for single file if menu extraction failed or no property item found
            if paths.len() == 1 {
                let _ = crate::infrastructure::windows::native_menu::show_properties_dialog(
                    hwnd,
                    &paths[0],
                );
            }
        }
    }

    pub fn create_new_folder(&mut self) {
        if self.is_computer_view || self.is_recycle_bin_view {
            return;
        }

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
                    self.scroll_to_selected = true;
                }

                if let Some(parent) = full_path.parent() {
                    self.directory_cache.invalidate(&parent.to_path_buf());
                    if let Some(directory_index) = &self.directory_index {
                        let _ = directory_index.invalidate(parent);
                    }
                }

                self.ui_ctx.request_repaint();
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
                self.file_ops_in_progress += 1;
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

    /// Monta uma ISO programaticamente e marca para auto-navegação
    pub fn mount_and_navigate_iso(&mut self, path: PathBuf) {
        use crate::infrastructure::windows::mount_iso;
        
        self.pending_iso_mount = Some(path.clone());
        
        match mount_iso(&path) {
            Ok(_) => {
                // Notifica o início da montagem
                self.notifications.push(crate::application::AppNotification::info(format!(
                    "Montando ISO: {}", 
                    path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                )));
            }
            Err(e) => {
                self.pending_iso_mount = None;
                self.notifications.push(crate::application::AppNotification::error(format!(
                    "Falha ao montar ISO: {}", e
                )));
            }
        }
    }
}
