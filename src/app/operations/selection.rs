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

            // Atualiza o MediaPreview
            let is_gif = selected.path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase() == "gif")
                .unwrap_or(false);

            if is_gif {
                use crate::ui::components::media_preview::GifPlayer;
                use crate::ui::components::media_preview::MediaPreview;
                
                // Só recarrega se for um caminho diferente do que já está no player
                let should_load = match &self.media_preview {
                    Some(MediaPreview::AnimatedGif(_)) => true, // Simplificado: recarrega sempre por enquanto
                    _ => true,
                };

                if should_load {
                    if let Ok(player) = GifPlayer::load(&self.ui_ctx, &selected.path) {
                        self.media_preview = Some(MediaPreview::AnimatedGif(player));
                    } else {
                        self.media_preview = None;
                    }
                }
            } else if let Some(tex) = self.cache_manager.texture_cache.peek(&selected.path) {
                use crate::ui::components::media_preview::MediaPreview;
                self.selected_thumbnail = Some(tex.clone());
                self.media_preview = Some(MediaPreview::StaticImage(tex.clone()));
            } else {
                // Se mudou de seleção e não tem no cache, limpa
                self.selected_thumbnail = None;
                self.media_preview = None;
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
        self.media_preview = None;
        self.selected_metadata = None;
        self.search_query.clear();
        self.context_menu.target_path = None;
        self.renaming_state = None;
    }
}
