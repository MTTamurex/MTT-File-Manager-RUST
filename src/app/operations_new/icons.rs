//! Icon loading and caching
//!
//! This module ensures standard icons (folder, computer) are loaded into the cache.

use eframe::egui;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows as windows_infra;

impl ImageViewerApp {
    pub fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };

        self.cache_manager
            .ensure_folder_icon(ctx, || windows_infra::extract_folder_icon(icon_size));
    }

    /// Garante que ícone de "Este Computador" está carregado.
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        self.cache_manager.ensure_computer_icon(ctx, || {
            windows_infra::extract_computer_icon(IconSize::Small)
        });
    }
}
