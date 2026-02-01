//! Tab synchronization
//!
//! This module handles syncing state between the active tab and the main application state.

use std::path::Path;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn sync_to_tab(&mut self) {
        let active = self.tab_manager.active_mut();
        active.path = self.current_path.clone();
        active.path_input = self.path_input.clone();
        active.is_computer_view = self.is_computer_view;
        active.is_recycle_bin_view = self.is_recycle_bin_view;
        active.navigation = self.navigation.clone();
        active.items = self.items.clone();
        // PERF: Move instead of clone to reduce memory duplication
        active.all_items = std::mem::take(&mut self.all_items);
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
            self.selected_thumbnail = active.selected_thumbnail.clone();
            self.selected_gif = active.selected_gif.clone();
            self.selected_metadata = active.selected_metadata.clone();
            self.search_query = active.search_query.clone();
            self.scroll_to_selected = active.scroll_to_selected;
            self.scroll_offset_y = active.scroll_offset_y;
            self.total_items = active.total_items;
        }

        self.watch_current_folder();
        
        // If items were cleared (by MoveCompleted event) and this is a regular folder view,
        // trigger a reload to fetch fresh content
        let needs_reload = self.items.is_empty() 
            && !self.is_computer_view 
            && !self.is_recycle_bin_view
            && !self.current_path.is_empty();
        
        if needs_reload {
            eprintln!("[TAB] Detected cleared items cache, reloading folder: {}", self.current_path);
            // Reset loaded_path to bypass the guard in load_folder
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

}
