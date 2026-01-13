//! User preferences save/load
//!
//! This module handles saving application state to the SQLite database.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};

impl ImageViewerApp {
    /// Salva as preferências atuais no SQLite
    pub fn save_preferences(&self) {
        let sort_mode_str = match self.sort_mode {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
        };
        self.disk_cache.set_preference("sort_mode", sort_mode_str);

        self.disk_cache.set_preference(
            "sort_descending",
            if self.sort_descending {
                "true"
            } else {
                "false"
            },
        );

        let folders_pos_str = match self.folders_position {
            FoldersPosition::First => "first",
            FoldersPosition::Last => "last",
            FoldersPosition::Mixed => "mixed",
        };
        self.disk_cache
            .set_preference("folders_position", folders_pos_str);

        // UI preferences
        self.disk_cache
            .set_preference("thumbnail_size", &self.thumbnail_size.to_string());

        let view_mode_str = match self.view_mode {
            ViewMode::Grid => "grid",
            ViewMode::List => "list",
        };
        self.disk_cache.set_preference("view_mode", view_mode_str);

        self.disk_cache.set_preference(
            "show_preview_panel",
            if self.show_preview_panel {
                "true"
            } else {
                "false"
            },
        );

        // Window state persistence
        self.disk_cache
            .set_preference("window_width", &self.saved_window_width.to_string());
        self.disk_cache
            .set_preference("window_height", &self.saved_window_height.to_string());
        self.disk_cache.set_preference(
            "window_is_maximized",
            if self.saved_is_maximized {
                "true"
            } else {
                "false"
            },
        );

        // Sidebar widths persistence - só salva valores válidos
        let left_to_save = self.sidebar_left_width.max(150.0);
        let right_to_save = self.sidebar_right_width.max(250.0);
        self.disk_cache
            .set_preference("sidebar_left_width", &left_to_save.to_string());
        self.disk_cache
            .set_preference("sidebar_right_width", &right_to_save.to_string());
    }
}
