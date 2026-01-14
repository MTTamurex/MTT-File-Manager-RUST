//! Selection state management
//!
//! This module handles updates to the selected item, including thumbnail syncing and clearing selection state.
//!
//! IMPORTANT: Media preview has owner-based protection. Only the owner tab can modify playback state.
//! Non-owner tabs can change their own selection without affecting the global media player.

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn update_selected_thumbnail(&mut self) {
        // Get active tab ID for ownership tracking
        let tab_id = self.tab_manager.active().id;
        
        // OWNER CHECK: Only clear/modify media_preview if:
        // 1. No owner exists (no media playing), OR
        // 2. Current tab IS the owner (can replace its own media)
        // Non-owner tabs selecting items will NOT affect playback
        let is_owner_or_none = self.media_preview_owner_tab_id.map_or(true, |id| id == tab_id);
        
        // Always reset thumbnail (visual only, not playback)
        self.selected_thumbnail = None;
        
        // Only clear media_preview if this tab owns it or no owner exists
        if is_owner_or_none {
            self.media_preview = None;
            self.media_preview_owner_tab_id = None;
        }

        if let Some(selected) = &self.selected_file {
            // Validate path exists before trying to load thumbnail
            if !selected.path.exists() {
                self.selected_file = None;
                return;
            }

            let ext = selected.path.extension()
                .map(|ext| ext.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            let is_gif = ext == "gif";
            let is_video = crate::infrastructure::windows::is_video_extension(&ext);

            // OWNER CHECK: Only load new media if this tab can take ownership
            // (is owner or no owner exists)
            if is_owner_or_none {
                if is_gif {
                    use crate::ui::components::media_preview::GifPlayer;
                    use crate::ui::components::media_preview::MediaPreview;
                    
                    if let Ok(player) = GifPlayer::load(&self.ui_ctx, &selected.path) {
                        self.media_preview = Some(MediaPreview::AnimatedGif(player));
                        self.media_preview_owner_tab_id = Some(tab_id);
                    }
                } else if is_video {
                    use crate::ui::components::WebviewPreview;
                    use crate::ui::components::media_preview::MediaPreview;

                    self.media_preview = Some(MediaPreview::Video(WebviewPreview::new(selected.path.clone())));
                    self.media_preview_owner_tab_id = Some(tab_id);
                } else if let Some(tex) = self.cache_manager.texture_cache.peek(&selected.path) {
                    use crate::ui::components::media_preview::MediaPreview;
                    self.selected_thumbnail = Some(tex.clone());
                    self.media_preview = Some(MediaPreview::StaticImage(tex.clone()));
                    self.media_preview_owner_tab_id = Some(tab_id);
                }
            } else {
                // Non-owner tab: just update thumbnail for static images (visual only)
                if !is_gif && !is_video {
                    if let Some(tex) = self.cache_manager.texture_cache.peek(&selected.path) {
                        self.selected_thumbnail = Some(tex.clone());
                    }
                }
            }
        }
    }

    /// Limpa a seleção atual, o thumbnail persistente, metadados e a busca.
    /// Útil durante navegação entre pastas.
    /// NOTE: Only clears media_preview if current tab is the owner.
    pub fn reset_selection_and_search(&mut self) {
        let tab_id = self.tab_manager.active().id;
        let is_owner = self.media_preview_owner_tab_id == Some(tab_id);
        
        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        
        // Only clear media if this tab owns it
        if is_owner || self.media_preview_owner_tab_id.is_none() {
            self.media_preview = None;
            self.media_preview_owner_tab_id = None;
        }
        
        self.selected_metadata = None;
        self.search_query.clear();
        self.context_menu.target_path = None;
        self.renaming_state = None;
    }

    /// Control WebView visibility based on current tab ownership.
    /// Shows video only when current tab is the owner, hides otherwise.
    /// Audio continues playing when hidden (video only hidden visually).
    /// This NEVER pauses, stops, or clears the media - just visual hide/show.
    pub fn update_video_visibility(&mut self) {
        use crate::ui::components::media_preview::MediaPreview;
        
        let active_tab_id = self.tab_manager.active().id;
        let is_owner = self.media_preview_owner_tab_id == Some(active_tab_id);
        
        if let Some(MediaPreview::Video(ref webview)) = self.media_preview {
            // Use WebView's set_visibility to show/hide based on ownership
            // This does NOT pause audio - just hides the video visually
            webview.set_visibility(is_owner);
        }
    }
}
