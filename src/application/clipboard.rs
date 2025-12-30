//! Clipboard state management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::PathBuf;

/// Clipboard operation type
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ClipboardOp {
    Copy,
    Move,
}

/// Clipboard state
#[derive(Clone, Debug)]
pub struct ClipboardState {
    pub file: Option<PathBuf>,
    pub operation: Option<ClipboardOp>,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            file: None,
            operation: None,
        }
    }
}

impl ClipboardState {
    /// Creates a new empty clipboard state
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Checks if clipboard has content
    pub fn has_content(&self) -> bool {
        self.file.is_some() && self.operation.is_some()
    }
    
    /// Gets clipboard state for paste operation
    pub fn get_for_paste(&self) -> Option<(&PathBuf, ClipboardOp)> {
        self.file.as_ref()
            .and_then(|file| self.operation.map(|op| (file, op)))
    }
    
    /// Clears the clipboard
    pub fn clear(&mut self) {
        self.file = None;
        self.operation = None;
    }
    
    /// Sets clipboard content
    pub fn set(&mut self, file: PathBuf, operation: ClipboardOp) {
        self.file = Some(file);
        self.operation = Some(operation);
    }
}
