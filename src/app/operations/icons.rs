//! Icon loading and caching
//!
//! This module ensures standard icons (folder, computer) are loaded into the cache.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows as windows_infra;
use eframe::egui;

impl ImageViewerApp {
    pub fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size > 120.0 {
            IconSize::Jumbo
        } else if thumbnail_size > 64.0 {
            IconSize::Large
        } else {
            IconSize::Small
        };

        self.cache_manager
            .ensure_folder_icon(ctx, || windows_infra::extract_folder_icon(icon_size));
    }

    /// Ensures the "This PC" icon is loaded.
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        self.cache_manager.ensure_computer_icon(ctx, || {
            windows_infra::extract_computer_icon(IconSize::Small)
        });
    }
}
