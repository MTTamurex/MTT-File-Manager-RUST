//! Item slot bridge - connects App state to item_slot component
//!
//! This module handles rendering individual item slots.

use eframe::egui;

use crate::app::state::ImageViewerApp;
use crate::ui::components::item_slot::{render_item_slot, ItemSlotContext, ItemSlotOperations};

impl ImageViewerApp {
    /// Render a single item slot (used for drag preview, etc.)
    pub fn render_item_slot(&mut self, ui: &mut egui::Ui, idx: usize) {
        if idx >= self.items.len() {
            return;
        }

        // Clone Arc to avoid borrowing self.items, allowing us to borrow the item
        // without a deep clone while still mutating self later
        let items_arc = self.items.clone();
        let item = &items_arc[idx];
        let is_renaming = self
            .renaming_state
            .as_ref()
            .map_or(false, |(i, _)| *i == idx);

        // To avoid borrow conflicts, collect pending operations
        // and execute after rendering
        let mut pending_thumbnail_loads: Vec<(std::path::PathBuf, u32, Option<usize>, u64)> = Vec::new();
        let mut pending_folder_scans: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_folder_preview_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_icon_loads: Vec<std::path::PathBuf> = Vec::new();
        let mut pending_rename: Option<usize> = None;

        // Rename text needs to be handled separately
        let mut renaming_text_clone = if is_renaming {
            self.renaming_state.as_ref().map(|(_, s)| s.clone())
        } else {
            None
        };

        // Create context with mutable reference to the clone
        {
            let renaming_text = renaming_text_clone.as_mut();

            let mut ctx = ItemSlotContext {
                item,
                idx,
                thumbnail_size: self.thumbnail_size,
                is_renaming,
                renaming_text,
                focus_rename: self.focus_rename,
                is_recycle_bin_view: self.is_recycle_bin_view,
                texture_cache: &mut self.cache_manager.texture_cache,
                icon_loader: &mut self.item_icon_loader,
                scanned_folders: &mut self.scanned_folders,
                loading_set: &mut self.cache_manager.loading_set,
                loading_icons: &mut self.loading_icons,
                failed_icons: &self.failed_icons,
                folder_preview_cache: &mut self.cache_manager.folder_preview_cache,
                folder_preview_loading: &mut self.cache_manager.folder_preview_loading,
                failed_thumbnails: &self.cache_manager.failed_thumbnails,
                pending_upload_set: &mut self.cache_manager.pending_upload_set,
                is_dense_mode: false,
                is_scrolling: false,
            };

            // Create simple ops struct that collects operations
            struct SimpleOps<'a> {
                thumbnail_loads: &'a mut Vec<(std::path::PathBuf, u32, Option<usize>, u64)>,
                folder_scans: &'a mut Vec<std::path::PathBuf>,
                folder_preview_loads: &'a mut Vec<std::path::PathBuf>,
                icon_loads: &'a mut Vec<std::path::PathBuf>,
                pending_rename: &'a mut Option<usize>,
            }

            impl<'a> ItemSlotOperations for SimpleOps<'a> {
                fn request_thumbnail_load(
                    &mut self,
                    path: std::path::PathBuf,
                    size: u32,
                    directory_index: Option<usize>,
                    modified: u64,
                ) {
                    self.thumbnail_loads.push((path, size, directory_index, modified));
                }

                fn request_folder_scan(&mut self, path: std::path::PathBuf) {
                    self.folder_scans.push(path);
                }

                fn request_folder_preview_load(&mut self, path: std::path::PathBuf) {
                    self.folder_preview_loads.push(path);
                }

                fn request_icon_load(&mut self, path: std::path::PathBuf) {
                    self.icon_loads.push(path);
                }

                fn rename_item(&mut self, idx: usize) {
                    *self.pending_rename = Some(idx);
                }
            }

            let mut ops = SimpleOps {
                thumbnail_loads: &mut pending_thumbnail_loads,
                folder_scans: &mut pending_folder_scans,
                folder_preview_loads: &mut pending_folder_preview_loads,
                icon_loads: &mut pending_icon_loads,
                pending_rename: &mut pending_rename,
            };

            let item_w = self.thumbnail_size;
            let item_h = self.thumbnail_size + 24.0; // Margin + Text
            let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(item_w, item_h));
            render_item_slot(ui, rect, &mut ctx, &mut ops);
        }

        // Apply changes after render
        if let Some(new_text) = renaming_text_clone {
            if is_renaming {
                if let Some((_, ref mut text)) = self.renaming_state {
                    *text = new_text;
                }
            }
        }

        // Execute pending operations
        for (path, size, index, modified) in pending_thumbnail_loads {
            if let Some(index) = index {
                self.request_thumbnail_load_with_index_and_modified(path, size, index, modified);
            } else {
                self.request_thumbnail_load_with_modified(path, size, modified);
            }
        }

        for path in pending_folder_scans {
            self.request_folder_scan(path);
        }

        for path in pending_folder_preview_loads {
            self.request_folder_preview_load(path);
        }

        for path in pending_icon_loads {
            self.request_icon_load(path);
        }

        if let Some(rename_idx) = pending_rename {
            self.rename_with_shell(rename_idx);
        }

        // Reset focus flag after first use
        if self.focus_rename {
            self.focus_rename = false;
        }
    }
}
