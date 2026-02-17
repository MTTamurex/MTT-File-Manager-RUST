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
        if self.should_skip_folder_load(force_refresh) {
            return;
        }
        self.mark_folder_load_started(force_refresh);
        self.bump_folder_load_generation();

        self.reset_folder_loading_state(force_refresh);

        self.start_folder_load_pipeline(force_refresh);
    }
}
