use crate::application::file_operations;
use crate::infrastructure::windows::shell_operations;
use crate::infrastructure::windows_clipboard;
use std::path::PathBuf;
use windows::Win32::Foundation::HWND;

/// Clipboard operation type
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ClipboardOp {
    Copy,
    Move,
}

/// Manages clipboard content and operations
#[derive(Clone, Debug)]
pub struct ClipboardManager {
    /// Internal clipboard state (fallback/cache)
    internal_file: Option<PathBuf>,
    internal_op: Option<ClipboardOp>,
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self {
            internal_file: None,
            internal_op: None,
        }
    }
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Helper to get internal state (read-only)
    pub fn internal_state(&self) -> (Option<&PathBuf>, Option<ClipboardOp>) {
        (self.internal_file.as_ref(), self.internal_op)
    }

    /// Checks if there is content to paste (System or Internal)
    pub fn has_content(&self) -> bool {
        windows_clipboard::has_files_in_clipboard() || self.internal_file.is_some()
    }

    /// Clears the internal clipboard state
    pub fn clear(&mut self) {
        self.internal_file = None;
        self.internal_op = None;
    }

    /// Copy a file to clipboard (System + Internal)
    pub fn copy(&mut self, path: &PathBuf) {
        // 1. System Clipboard
        let _ = file_operations::copy_path_to_clipboard(path);

        // 2. Internal State
        self.internal_file = Some(path.clone());
        self.internal_op = Some(ClipboardOp::Copy);
    }

    /// Cut a file (System + Internal)
    pub fn cut(&mut self, path: &PathBuf) {
        // 1. System Clipboard (Cut usually sets DropEffect, but for now we copy path)
        // Ideally we should use OLE clipboard with DropEffect::Move,
        // but existing impl just copies path.
        let _ = file_operations::copy_path_to_clipboard(path);

        // 2. Internal State
        self.internal_file = Some(path.clone());
        self.internal_op = Some(ClipboardOp::Move);
    }

    /// Paste files to destination
    /// Returns: Ok(true) if files were moved (source should be cleared), Ok(false) if copied.
    pub fn paste(&mut self, dest_folder: &PathBuf, hwnd: Option<HWND>) -> Result<bool, String> {
        let hwnd = hwnd.unwrap_or(HWND(std::ptr::null_mut()));

        // 1. Try System Clipboard first
        if let Some(files) = windows_clipboard::get_files_from_clipboard() {
            let op = windows_clipboard::get_clipboard_operation();
            let is_move = matches!(op, Some(windows_clipboard::ClipboardFileOp::Move));

            self.execute_paste(files, is_move, dest_folder, hwnd)
        } else if let Some(path) = &self.internal_file {
            // 2. Fallback to Internal
            let is_move = matches!(self.internal_op, Some(ClipboardOp::Move));
            let files = vec![path.clone()];

            let result = self.execute_paste(files, is_move, dest_folder, hwnd)?;

            if result && is_move {
                self.internal_file = None;
                self.internal_op = None;
            }
            Ok(result)
        } else {
            Err("Área de transferência vazia".to_string())
        }
    }

    fn execute_paste(
        &self,
        files: Vec<PathBuf>,
        is_move: bool,
        dest_folder: &PathBuf,
        hwnd: HWND,
    ) -> Result<bool, String> {
        let mut any_success = false;

        for src_path in files {
            // Skip logic is inside shell_operations helper for move, but explicit check implies intention
            if is_move {
                if shell_operations::move_item_with_shell(&src_path, dest_folder, hwnd) {
                    any_success = true;
                }
            } else {
                if shell_operations::copy_item_with_shell(&src_path, dest_folder, hwnd) {
                    any_success = true;
                }
            }
        }

        Ok(any_success)
    }
}
