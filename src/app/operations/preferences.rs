//! User preferences save/load
//!
//! This module handles saving application state to the SQLite database.
//!
//! PERFORMANCE: `save_preferences()` is debounced — it sets a dirty flag and the
//! actual write happens in `flush_preferences_if_needed()` which runs once per frame
//! but only writes to disk if >1 second has passed since the last write. This prevents
//! 20+ synchronous SQLite writes from blocking the UI thread on state changes.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};

/// Minimum interval between actual disk writes
const PREFERENCES_FLUSH_INTERVAL_MS: u64 = 1000;

impl ImageViewerApp {
    /// Marks preferences as dirty (deferred write).
    /// The actual SQLite writes happen in `flush_preferences_if_needed()`.
    pub fn save_preferences(&mut self) {
        self.preferences_dirty = true;
    }

    /// Flushes dirty preferences to SQLite if enough time has passed.
    /// Called once per frame from the update loop.
    pub fn flush_preferences_if_needed(&mut self) {
        if !self.preferences_dirty {
            return;
        }
        if self.preferences_last_save.elapsed().as_millis() < PREFERENCES_FLUSH_INTERVAL_MS as u128
        {
            return;
        }
        self.preferences_dirty = false;
        self.preferences_last_save = std::time::Instant::now();
        self.do_save_preferences();
    }

    /// Force-flushes preferences immediately (for exit).
    pub fn force_save_preferences(&self) {
        self.do_save_preferences();
    }

    /// Actually writes all preferences to SQLite.
    fn do_save_preferences(&self) {
        let sort_mode_str = match self.sort_mode {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
            SortMode::DriveTotalSpace => "drive_total",
            SortMode::DriveFreeSpace => "drive_free",
        };
        self.disk_cache.set_preference("sort_mode", sort_mode_str);

        let sort_mode_computer_str = match self.sort_mode_computer {
            SortMode::Name => "name",
            SortMode::DriveTotalSpace => "drive_total",
            SortMode::DriveFreeSpace => "drive_free",
            _ => "name", // Computer view only supports these 3
        };
        self.disk_cache
            .set_preference("sort_mode_computer", sort_mode_computer_str);

        let sort_mode_normal_str = match self.sort_mode_normal {
            SortMode::Name => "name",
            SortMode::Date => "date",
            SortMode::Size => "size",
            SortMode::Type => "type",
            _ => "name", // Normal folders don't use drive modes
        };
        self.disk_cache
            .set_preference("sort_mode_normal", sort_mode_normal_str);

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
        self.disk_cache
            .set_preference("upload_budget_ms", &self.upload_budget_ms.to_string());

        // Window state persistence
        self.disk_cache
            .set_preference("window_width", &self.layout.saved_window_width.to_string());
        self.disk_cache
            .set_preference("window_height", &self.layout.saved_window_height.to_string());
        self.disk_cache.set_preference(
            "window_is_maximized",
            if self.layout.saved_is_maximized {
                "true"
            } else {
                "false"
            },
        );

        // Sidebar widths persistence - only save valid values
        let left_to_save = self.layout.sidebar_left_width.max(150.0);
        let right_to_save = self.layout.sidebar_right_width.max(250.0);
        self.disk_cache
            .set_preference("sidebar_left_width", &left_to_save.to_string());
        self.disk_cache
            .set_preference("sidebar_right_width", &right_to_save.to_string());

        // Save last active folder from current tab
        let last_folder = self.tab_manager.active().path.clone();
        // Only save if it's a real path (not "Este Computador" or "Lixeira")
        if !last_folder.is_empty()
            && last_folder != "Este Computador"
            && last_folder != "Lixeira"
            && !last_folder.starts_with("shell:")
        {
            self.disk_cache.set_preference("last_folder", &last_folder);
        }

        // Save media player volume if available
        if let Some(preview) = &self.media_preview {
            if let Some(volume) = preview.get_video_state().map(|s| s.volume) {
                self.disk_cache
                    .set_preference("media_volume", &volume.to_string());
            }
        }

        // Save list view column widths - Regular view
        self.disk_cache
            .set_preference("list_col_name_width", &self.layout.list_col_name_width.to_string());
        self.disk_cache
            .set_preference("list_col_date_width", &self.layout.list_col_date_width.to_string());
        self.disk_cache
            .set_preference("list_col_type_width", &self.layout.list_col_type_width.to_string());
        self.disk_cache
            .set_preference("list_col_size_width", &self.layout.list_col_size_width.to_string());
        // Save list view column widths - OneDrive view
        self.disk_cache.set_preference(
            "list_col_onedrive_name_width",
            &self.layout.list_col_onedrive_name_width.to_string(),
        );
        self.disk_cache.set_preference(
            "list_col_onedrive_date_width",
            &self.layout.list_col_onedrive_date_width.to_string(),
        );
        self.disk_cache.set_preference(
            "list_col_onedrive_type_width",
            &self.layout.list_col_onedrive_type_width.to_string(),
        );
        self.disk_cache.set_preference(
            "list_col_onedrive_size_width",
            &self.layout.list_col_onedrive_size_width.to_string(),
        );
        self.disk_cache.set_preference(
            "list_col_onedrive_status_width",
            &self.layout.list_col_onedrive_status_width.to_string(),
        );
        // Save list view column widths - Computer view
        self.disk_cache.set_preference(
            "list_col_computer_name_width",
            &self.layout.list_col_computer_name_width.to_string(),
        );
        self.disk_cache.set_preference(
            "list_col_computer_total_width",
            &self.layout.list_col_computer_total_width.to_string(),
        );
        self.disk_cache.set_preference(
            "list_col_computer_free_width",
            &self.layout.list_col_computer_free_width.to_string(),
        );
    }
}
