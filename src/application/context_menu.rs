//! Context menu state management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::PathBuf;

use eframe::egui;

/// Context menu state
#[derive(Clone, Debug)]
pub struct ContextMenuState {
    pub is_open: bool,
    pub position: egui::Pos2,
    pub item_index: Option<usize>,
    pub target_path: Option<PathBuf>,
    pub is_empty_area: bool,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            is_open: false,
            position: egui::Pos2::ZERO,
            item_index: None,
            target_path: None,
            is_empty_area: false,
        }
    }
}

impl ContextMenuState {
    /// Creates a new context menu state
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Opens the context menu at the specified position
    pub fn open(&mut self, position: egui::Pos2, item_index: Option<usize>, target_path: Option<PathBuf>, is_empty_area: bool) {
        self.is_open = true;
        self.position = position;
        self.item_index = item_index;
        self.target_path = target_path;
        self.is_empty_area = is_empty_area;
    }
    
    /// Closes the context menu
    pub fn close(&mut self) {
        self.is_open = false;
        self.item_index = None;
        self.target_path = None;
        self.is_empty_area = false;
    }
    
    /// Checks if the context menu is open for a specific item
    pub fn is_open_for_item(&self, index: usize) -> bool {
        self.is_open && self.item_index == Some(index)
    }
    
    /// Checks if the context menu is open for empty area
    pub fn is_open_for_empty_area(&self) -> bool {
        self.is_open && self.is_empty_area
    }
}
