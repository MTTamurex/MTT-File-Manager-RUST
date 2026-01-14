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
        active.all_items = self.all_items.clone();
        active.selected_item = self.selected_item;
        active.selected_file = self.selected_file.clone();
        active.selected_thumbnail = self.selected_thumbnail.clone();
        active.selected_metadata = self.selected_metadata.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;

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
        // Clonamos o estado da aba para evitar problemas de borrow checker ao atualizar self
        let active = self.tab_manager.active().clone();
        self.current_path = active.path;
        self.path_input = active.path_input;
        self.is_computer_view = active.is_computer_view;
        self.is_recycle_bin_view = active.is_recycle_bin_view;
        self.navigation = active.navigation.clone();
        self.items = active.items;
        self.all_items = active.all_items;
        self.selected_item = active.selected_item;
        self.selected_file = active.selected_file;
        self.selected_thumbnail = active.selected_thumbnail;
        self.selected_metadata = active.selected_metadata;
        self.search_query = active.search_query;
        self.scroll_to_selected = active.scroll_to_selected;

        self.watch_current_folder();
    }
}
