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
    /// Pending native shell context menu - path, screen coordinates, and start timestamp
    /// The menu will only be shown after enough time has passed for the UI to be rendered.
    /// Format: (path, screen_x, screen_y, start_time_ms) where start_time_ms is 0 initially
    pub pending_native_menu: Option<(PathBuf, i32, i32, u64)>,
    /// Flag to indicate that a right-click selection was made and the menu should open
    /// after the selection has been drawn. This ensures visual feedback before the menu appears.
    pub needs_draw_before_menu: bool,
    /// Pending click to replay after menu was dismissed by clicking outside
    /// Format: (screen_x, screen_y, is_right_click) - stored when menu is cancelled by outside click
    pub pending_click_replay: Option<(i32, i32, bool)>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            is_open: false,
            position: egui::Pos2::ZERO,
            item_index: None,
            target_path: None,
            is_empty_area: false,
            pending_native_menu: None,
            needs_draw_before_menu: false,
            pending_click_replay: None,
        }
    }
}

impl ContextMenuState {
    /// Creates a new context menu state
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the context menu at the specified position
    pub fn open(
        &mut self,
        position: egui::Pos2,
        item_index: Option<usize>,
        target_path: Option<PathBuf>,
        is_empty_area: bool,
    ) {
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
