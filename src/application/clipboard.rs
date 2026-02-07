use crate::application::file_operations;
use crate::infrastructure::windows_clipboard;
use std::path::PathBuf;

/// Clipboard operation type
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ClipboardOp {
    Copy,
    Move,
}

/// Manages clipboard content and operations
#[derive(Clone, Debug, Default)]
pub struct ClipboardManager {
    /// Internal clipboard state (fallback/cache)
    internal_files: Vec<PathBuf>,
    internal_op: Option<ClipboardOp>,
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Helper to get internal state (read-only)
    pub fn internal_state(&self) -> (&[PathBuf], Option<ClipboardOp>) {
        (&self.internal_files, self.internal_op)
    }

    /// Checks if there is content to paste (System or Internal)
    pub fn has_content(&self) -> bool {
        windows_clipboard::has_files_in_clipboard() || !self.internal_files.is_empty()
    }

    /// Clears the internal clipboard state
    pub fn clear(&mut self) {
        self.internal_files.clear();
        self.internal_op = None;
    }

    /// Copy files to clipboard (System + Internal)
    pub fn copy(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() { return; }

        // 1. System Clipboard (Just copy first path as text for now, should improve later)
        if let Some(first) = paths.first() {
            let _ = file_operations::copy_path_to_clipboard(first);
        }

        // 2. Internal State
        self.internal_files = paths.to_vec();
        self.internal_op = Some(ClipboardOp::Copy);
    }

    /// Cut files (System + Internal)
    pub fn cut(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() { return; }

        // 1. System Clipboard
        if let Some(first) = paths.first() {
            let _ = file_operations::copy_path_to_clipboard(first);
        }

        // 2. Internal State
        self.internal_files = paths.to_vec();
        self.internal_op = Some(ClipboardOp::Move);
    }

    /// Returns files and operation type (is_move) for pasting.
    /// Does NOT perform the operation. Use this to prepare an async operation.
    pub fn get_files_to_paste(&self) -> Option<(Vec<PathBuf>, bool)> {
        // 1. Try System Clipboard first
        if let Some(files) = windows_clipboard::get_files_from_clipboard() {
            let op = windows_clipboard::get_clipboard_operation();
            let is_move = matches!(op, Some(windows_clipboard::ClipboardFileOp::Move));
            return Some((files, is_move));
        }

        // 2. Fallback to Internal
        if !self.internal_files.is_empty() {
            let is_move = matches!(self.internal_op, Some(ClipboardOp::Move));
            return Some((self.internal_files.clone(), is_move));
        }

        None
    }
}
