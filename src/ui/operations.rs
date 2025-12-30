//! File operations for the file manager.
//!
//! This module handles file operations like copy, cut, paste, delete,
//! rename, and folder creation.

use std::path::PathBuf;

use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::UI::Shell::*,
};

use crate::domain::file_entry::FileEntry;
use crate::ui::app::{ClipboardOp, ImageViewerApp};

impl ImageViewerApp {
    /// Deletes selected file/folder using Shell API (supports Undo/Ctrl+Z).
    pub fn delete_with_shell(&mut self) {
        if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                let path = item.path.to_string_lossy().to_string();

                // Double-null termination required by API
                let mut from_vec: Vec<u16> = path.encode_utf16().collect();
                from_vec.push(0);
                from_vec.push(0);

                let mut op = SHFILEOPSTRUCTW {
                    hwnd: HWND(std::ptr::null_mut()),
                    wFunc: FO_DELETE,
                    pFrom: PCWSTR(from_vec.as_ptr()),
                    pTo: PCWSTR(std::ptr::null()),
                    fFlags: (FOF_ALLOWUNDO | FOF_WANTNUKEWARNING).0 as u16,
                    ..Default::default()
                };

                unsafe {
                    let result = SHFileOperationW(&mut op);
                    if result == 0 {
                        // Watcher will handle refresh, but we can clear selection
                        self.selected_item = None;
                        self.selected_file = None;
                    }
                }
            }
        }
    }

    /// Creates a new folder in the current directory.
    pub fn create_new_folder(&mut self) {
        let base_path = PathBuf::from(&self.current_path);
        let mut new_folder_name = "Nova Pasta".to_string();
        let mut counter = 1;

        while base_path.join(&new_folder_name).exists() {
            counter += 1;
            new_folder_name = format!("Nova Pasta ({})", counter);
        }

        let full_path = base_path.join(&new_folder_name);

        if std::fs::create_dir(&full_path).is_ok() {
            // CRITICAL: To rename immediately, we use helper from_path
            let new_item = FileEntry::from_path(full_path.clone(), true);
            
            self.all_items.push(new_item);
            self.filter_items();
            self.sort_items();

            // Find index in filtered vector (items)
            if let Some(idx) = self.items.iter().position(|i| i.path == full_path) {
                self.selected_item = Some(idx);
                self.selected_file = Some(self.items[idx].clone());
                self.renaming_state = Some((idx, new_folder_name));
                self.focus_rename = true;
            }
            
            // Requests real load in background to ensure sync with disk
            self.load_folder();
        }
    }
    
    /// Copy: Saves selected file path to memory.
    pub fn command_copy(&mut self) {
        if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                self.clipboard_file = Some(item.path.clone());
                self.clipboard_op = Some(ClipboardOp::Copy);
            }
        }
    }
    
    /// Cut: Saves selected file path with move flag.
    pub fn command_cut(&mut self) {
        if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                self.clipboard_file = Some(item.path.clone());
                self.clipboard_op = Some(ClipboardOp::Move);
            }
        }
    }
    
    /// Paste: Executes SHFileOperationW to copy or move file.
    pub fn command_paste(&mut self) {
        // 1. Validation: something to paste?
        let src_path = match &self.clipboard_file {
            Some(p) => p.clone(),
            None => { return; }
        };
        
        // 2. Determine destination folder: uses target_path from context menu if available and valid,
        // otherwise uses current_path (compatibility with keyboard shortcuts)
        let dest_folder = if let Some(target) = &self.context_menu_target_path {
            // Checks if target still exists (wasn't deleted)
            if target.exists() && target.is_dir() {
                target.clone()
            } else {
                // If target no longer exists, uses current_path and clears target
                self.context_menu_target_path = None;
                PathBuf::from(&self.current_path)
            }
        } else {
            PathBuf::from(&self.current_path)
        };
        
        // 3. Check if source file already exists in destination folder
        if let Some(file_name) = src_path.file_name() {
            let dest_file = dest_folder.join(file_name);
            if dest_file.exists() && dest_file != src_path {
                // Windows will show replace dialog, but we can prevent redundant operation
                // If moving to same folder (same file), do nothing
                if let Some(ClipboardOp::Move) = self.clipboard_op {
                    if src_path.parent() == Some(&dest_folder) {
                        return;
                    }
                }
            }
        }
        
        // 4. Avoid moving to same folder (redundant)
        if let Some(ClipboardOp::Move) = self.clipboard_op {
            if src_path.parent() == Some(&dest_folder) {
                return;
            }
        }
        
        // 5. Prepare strings for Windows API (double-null terminated)
        let mut from_vec: Vec<u16> = src_path.to_string_lossy().encode_utf16().collect();
        from_vec.push(0);
        from_vec.push(0);
        
        let mut to_vec: Vec<u16> = dest_folder.to_string_lossy().encode_utf16().collect();
        to_vec.push(0);
        to_vec.push(0);
        
        // 6. Define operation (FO_COPY or FO_MOVE)
        let w_func = match self.clipboard_op {
            Some(ClipboardOp::Move) => FO_MOVE,
            _ => FO_COPY,
        };
        
        let mut op = SHFILEOPSTRUCTW {
            hwnd: HWND(std::ptr::null_mut()),
            wFunc: w_func,
            pFrom: PCWSTR(from_vec.as_ptr()),
            pTo: PCWSTR(to_vec.as_ptr()),
            fFlags: (FOF_ALLOWUNDO).0 as u16,
            ..Default::default()
        };
        
        // 7. Execute operation
        unsafe {
            let result = SHFileOperationW(&mut op);
            
            if result == 0 {
                // If was Cut, clear clipboard
                if let Some(ClipboardOp::Move) = self.clipboard_op {
                    self.clipboard_file = None;
                    self.clipboard_op = None;
                }
                // Reload folder to see result
                self.load_folder();
            }
        }
        
        // 8. Clear context_menu_target_path after operation
        self.context_menu_target_path = None;
    }
    
    /// Renames file using Shell API (supports Undo/Ctrl+Z).
    pub fn rename_with_shell(&mut self, idx: usize) {
        if let Some((_, new_name)) = self.renaming_state.take() {
            if let Some(item) = self.items.get(idx) {
                let old_path = item.path.to_string_lossy().to_string();
                if let Some(parent) = item.path.parent() {
                    let new_path = parent.join(&new_name).to_string_lossy().to_string();

                    // API rule: Strings must end with TWO nulls (\0\0)
                    let mut from_vec: Vec<u16> = old_path.encode_utf16().collect();
                    from_vec.push(0); 
                    from_vec.push(0);

                    let mut to_vec: Vec<u16> = new_path.encode_utf16().collect();
                    to_vec.push(0); 
                    to_vec.push(0);

                    let mut op = SHFILEOPSTRUCTW {
                        hwnd: HWND(std::ptr::null_mut()), 
                        wFunc: FO_RENAME,
                        pFrom: PCWSTR(from_vec.as_ptr()),
                        pTo: PCWSTR(to_vec.as_ptr()),
                        fFlags: FOF_ALLOWUNDO.0 as u16, 
                        ..Default::default()
                    };

                    unsafe {
                        let result = SHFileOperationW(&mut op);
                        if result == 0 {
                            // Success: Reload folder to update UI
                            self.load_folder();
                        } else {
                            eprintln!("Error renaming via Shell: {}", result);
                        }
                    }
                }
            }
        }
    }
}
