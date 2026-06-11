//! Folder loading: load_folder, filter_items, sort_items, refresh
//!
//! This module handles scanning folders, filtering results, sorting, and manual refresh triggers.

use crate::app::state::ImageViewerApp;

mod folder_scan;
mod guards;
mod load_pipeline;
mod refresh;
mod view_updates;

impl ImageViewerApp {
    pub fn load_folder(&mut self, force_refresh: bool) {
        self.load_folder_with_icon_trim(force_refresh, true);
    }

    pub fn reload_current_folder_preserving_icon_cache(&mut self) {
        self.load_folder_with_icon_trim(false, false);
    }

    fn load_folder_with_icon_trim(&mut self, force_refresh: bool, trim_icons: bool) {
        if self.should_skip_folder_load(force_refresh) {
            return;
        }
        self.mark_folder_load_started(force_refresh);
        self.bump_folder_load_generation();

        self.reset_folder_loading_state(force_refresh, trim_icons);

        self.start_folder_load_pipeline(force_refresh, true);
    }

    /// Lightweight folder load for the **inactive** dual panel.
    ///
    /// Unlike `load_folder`, this does NOT clear shared caches
    /// (`loading_set`, `pending_upload_set`, `pending_thumbnails`, icons,
    /// `scanned_folders`, etc.) which belong to the active panel's thumbnail
    /// pipeline.  Clearing them would nuke in-progress thumbnail work for the
    /// active panel and cause persistent thumbnail display corruption.
    pub fn load_folder_for_inactive(&mut self) {
        if self.should_skip_folder_load(false) {
            return;
        }
        self.mark_folder_load_started(false);
        self.bump_folder_load_generation();

        // Reset ONLY per-panel state; leave shared caches untouched.
        let preserve_visual_pipeline = !self.items.is_empty();
        self.pending_all_items_clear = true;
        self.hold_visible_items_until_load_complete = preserve_visual_pipeline;
        if !preserve_visual_pipeline {
            self.selected_item = None;
            self.total_items = 0;
        }
        self.is_loading_folder = true;
        self.folder_load_error = None;
        self.loading_started_at = std::time::Instant::now();
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.last_items_rebuild = std::time::Instant::now();

        self.start_folder_load_pipeline(false, false);
    }
}
