//! Tab synchronization
//!
//! This module handles syncing state between the active tab and the main application state.

use crate::app::state::ImageViewerApp;
use std::path::Path;

impl ImageViewerApp {
    pub fn sync_to_tab(&mut self) {
        let active = self.tab_manager.active_mut();
        active.path = self.current_path.clone();
        active.path_input = self.path_input.clone();
        active.is_computer_view = self.is_computer_view;
        active.is_recycle_bin_view = self.is_recycle_bin_view;
        active.navigation = self.navigation.clone();
        active.items = self.items.clone();
        // Special views still receive async in-place updates (ex: poll_drive_info for drives).
        // Moving all_items out of app state would make those updaters rebuild the UI from an empty list.
        if self.is_computer_view || self.is_recycle_bin_view {
            active.all_items = self.all_items.clone();
        } else {
            // PERF: Move instead of clone to reduce memory duplication
            active.all_items = std::mem::take(&mut self.all_items);
        }
        active.selected_item = self.selected_item;
        active.selected_file = self.selected_file.clone();
        // PERF: Keep thumbnail when syncing (user might return to this tab)
        active.selected_thumbnail = self.selected_thumbnail.clone();
        active.selected_gif = self.selected_gif.clone();
        active.selected_metadata = self.selected_metadata.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;
        active.scroll_offset_y = self.scroll_offset_y;
        active.total_items = self.total_items;
        active.view_mode = self.view_mode;
        // PERF: Move instead of clone for multi_selection (same pattern as all_items)
        active.multi_selection = std::mem::take(&mut self.multi_selection);
        active.sort_mode = self.sort_mode;
        active.sort_descending = self.sort_descending;
        active.folders_position = self.folders_position;

        // No Windows, Path::new("Este Computador").file_name() é None
        if active.is_computer_view {
            active.title = "Este Computador".to_string();
        } else {
            active.title = Path::new(&active.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| active.path.clone());
        }
    }

    /// Sincroniza o estado da aba ativa para o app
    pub fn sync_from_tab(&mut self) {
        {
            let active = self.tab_manager.active_mut();
            self.current_path = active.path.clone();
            self.path_input = active.path_input.clone();
            self.is_computer_view = active.is_computer_view;
            self.is_recycle_bin_view = active.is_recycle_bin_view;
            self.navigation = active.navigation.clone();
            self.items = active.items.clone();
            self.all_items = std::mem::take(&mut active.all_items);
            self.selected_item = active.selected_item;
            self.selected_file = active.selected_file.clone();

            // FIX: Validate that selected_file still exists in items
            // If the file was moved/deleted while on another tab, clear selection
            // so preview panel shows current folder info instead of stale data.
            // This is a pure in-memory check (no filesystem I/O).
            if let Some(ref selected) = self.selected_file {
                let still_exists = self.items.iter().any(|item| item.path == selected.path);
                if !still_exists {
                    // Store selected path before clearing for folder cover check
                    let removed_path = selected.path.clone();

                    self.selected_file = None;
                    self.selected_item = None;
                    self.selected_thumbnail = None;
                    self.selected_metadata = None;
                    // Also clear from tab so it doesn't come back on next sync
                    active.selected_file = None;
                    active.selected_item = None;
                    active.selected_thumbnail = None;
                    active.selected_metadata = None;

                    // FIX: If the removed file was the folder cover for this folder,
                    // invalidate the folder_preview_cache so the preview panel updates.
                    // Uses SQLite lookup (minimal I/O) and requests recalculation.
                    let current_folder = std::path::PathBuf::from(&active.path);
                    let covers = self
                        .disk_cache
                        .get_folder_covers(std::slice::from_ref(&current_folder));
                    if let Some(current_cover) = covers.get(&current_folder) {
                        if current_cover == &removed_path {
                            self.disk_cache.remove_folder_cover(&current_folder);
                            self.cache_manager.folder_preview_cache.pop(&current_folder);
                            let _ = self.cover_worker_sender.send(current_folder);
                        }
                    }
                }
            }

            self.selected_thumbnail = active.selected_thumbnail.clone();
            self.selected_gif = active.selected_gif.clone();
            self.selected_metadata = active.selected_metadata.clone();
            self.search_query = active.search_query.clone();
            self.scroll_to_selected = active.scroll_to_selected;
            self.scroll_offset_y = active.scroll_offset_y;
            self.total_items = active.total_items;
            self.view_mode = active.view_mode;
            self.multi_selection = std::mem::take(&mut active.multi_selection);
            self.sort_mode = active.sort_mode;
            self.sort_descending = active.sort_descending;
            self.folders_position = active.folders_position;
        }

        self.watch_current_folder();

        // If items were cleared (by MoveCompleted event) and this is a regular folder view,
        // trigger a reload to fetch fresh content
        let needs_reload = self.items.is_empty()
            && !self.is_computer_view
            && !self.is_recycle_bin_view
            && !self.current_path.is_empty();

        if needs_reload {
            eprintln!(
                "[TAB] Detected cleared items cache, reloading folder: {}",
                self.current_path
            );
            // Reset loaded_path to bypass the guard in load_folder
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }
}
