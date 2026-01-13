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
                // Delegate to application layer
                if let Ok(true) = file_operations::delete_with_shell(&item.path, self.native_hwnd) {
                    // Limpa cache do item deletado
                    self.disk_cache.remove_cache_for_path(&item.path);

                    // O watcher vai cuidar do refresh, mas podemos limpar a seleção
                    if self.selected_item == Some(idx) {
                        self.selected_item = None;
                        self.selected_file = None;
                    }
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

    /// Renomeia arquivo usando Shell API (suporta Undo/Ctrl+Z)
    pub fn rename_with_shell(&mut self, idx: usize) {
        if let Some((_, new_name)) = self.renaming_state.take() {
            if let Some(item) = self.items.get(idx) {
                let old_path = item.path.to_string_lossy().to_string();
                if let Some(parent) = item.path.parent() {
                    let new_path = parent.join(&new_name).to_string_lossy().to_string();

                    // Regra da API: Strings devem terminar com DOIS nulls (\0\0)
                    let mut from_vec: Vec<u16> = old_path.encode_utf16().collect();
                    from_vec.push(0);
                    from_vec.push(0);

                    let mut to_vec: Vec<u16> = new_path.encode_utf16().collect();
                    to_vec.push(0);
                    to_vec.push(0);

                    use windows::Win32::UI::Shell::{SHFileOperationW, SHFILEOPSTRUCTW, FO_RENAME, FOF_ALLOWUNDO};
                    use windows::core::PCWSTR;

                    let mut sh_file_op = SHFILEOPSTRUCTW {
                        wFunc: FO_RENAME,
                        pFrom: PCWSTR(from_vec.as_ptr()),
                        pTo: PCWSTR(to_vec.as_ptr()),
                        fFlags: FOF_ALLOWUNDO.0 as u16,
                        ..Default::default()
                    };

                    unsafe {
                        let _ = SHFileOperationW(&mut sh_file_op);
                    }
                    
                    // Atualiza UI
                    self.load_folder(false);
                }
            }
        }
    }

    /// Create a Windows shell shortcut (.lnk) pointing to `target` in the same directory.
    pub fn create_shell_shortcut(&self, target: &Path) -> Result<PathBuf, String> {
        file_operations::create_shortcut(target, &self.current_path)
    }
}
