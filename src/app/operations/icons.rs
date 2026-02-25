//! Icon loading and caching
//!
//! This module ensures standard icons (computer) are loaded into the cache.
//! Folder icon is pre-set at init from custom compose — no per-frame ensure needed.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows as windows_infra;
use eframe::egui;

impl ImageViewerApp {
    /// Ensures the "This PC" icon is loaded.
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        self.cache_manager.ensure_computer_icon(ctx, || {
            windows_infra::extract_computer_icon(IconSize::Small)
        });
    }
}
