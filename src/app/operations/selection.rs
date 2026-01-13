//! Selection state management
//!
//! This module handles updates to the selected item, including thumbnail syncing and clearing selection state.

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn update_selected_thumbnail(&mut self) {
        if let Some(selected) = &self.selected_file {
            // Validate path exists before trying to load thumbnail
            if !selected.path.exists() {
                self.selected_file = None;
                self.selected_thumbnail = None;
                return;
            }

            // Tenta pegar do cache. Se não estiver lá, mantém None (será atualizado via message loop)
            if let Some(tex) = self.cache_manager.texture_cache.peek(&selected.path) {
                self.selected_thumbnail = Some(tex.clone());
            } else {
                // Se mudou de seleção e não tem no cache, limpa
                self.selected_thumbnail = None;
            }
        } else {
            self.selected_thumbnail = None;
        }
    }

    /// Limpa a seleção atual, o thumbnail persistente, metadados e a busca.
    /// Útil durante navegação entre pastas.
    pub fn reset_selection_and_search(&mut self) {
        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        self.selected_metadata = None;
        self.search_query.clear();
        self.context_menu.target_path = None;
        self.renaming_state = None;
    }
}
