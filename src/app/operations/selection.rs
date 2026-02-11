//! Selection state management
//!
//! This module handles updates to the selected item, including thumbnail syncing and clearing selection state.
//!
//! IMPORTANT: Media preview has owner-based protection. Only the owner tab can modify playback state.
//! Non-owner tabs can change their own selection without affecting the global media player.

use crate::app::state::ImageViewerApp;
use crate::infrastructure::onedrive;

impl ImageViewerApp {
    /// Teardown media preview resources immediately (MPV buffers, thread, HWND).
    pub fn destroy_media_preview(&mut self) {
        if let Some(mut preview) = self.media_preview.take() {
            preview.shutdown();
        }
        self.media_preview_owner_tab_id = None;
        self.ui_ctx.request_repaint();
        // Run memory maintenance immediately after tearing down the player.
        self.run_memory_maintenance_now();
    }

    pub fn update_selected_thumbnail(&mut self) {
        // Selection change only updates the persistent thumbnail and metadata.
        // It NO LONGER clears or sets the global media_preview automatically.
        // This allows playback to continue in the background while the user browses.

        self.selected_thumbnail = None;
        self.selected_gif = None;

        if let Some(selected) = &self.selected_file {
            let path = selected.path.clone();
            let modified = selected.modified;
            let is_media = selected.is_media();
            // Validate path exists before trying to load thumbnail
            // Skip this check for virtual paths (files inside ZIP archives)
            let is_virtual_path =
                crate::infrastructure::windows::shell_folder::is_shell_navigation_path(
                    &path, false,
                );
            // CRITICAL FIX: Use fast_path_exists() instead of path.exists()
            // path.exists() uses CreateFileW which triggers OneDrive file recall,
            // blocking the UI thread for 30-60s on cloud-only files.
            // GetFileAttributesW reads cached attributes — no network I/O.
            if !is_virtual_path && !onedrive::fast_path_exists(&path) {
                self.selected_file = None;
                self.update_video_visibility(); // Sync visibility after clearing selection
                return;
            }

            // Keep currently available texture, but only request 512px when the existing one
            // is missing or smaller than needed for the detail panel.
            let has_required_texture =
                if let Some(tex) = self.cache_manager.texture_cache.peek(&path) {
                    self.selected_thumbnail = Some(tex.clone());
                    let tex_size = tex.size();
                    (tex_size[0].max(tex_size[1]) as u32) >= 512
                } else {
                    false
                };

            // Avoid re-request loops: once 512px exists (or is already in-flight/pending upload),
            // selection changes should not enqueue extraction again.
            if is_media
                && !has_required_texture
                && !self.cache_manager.is_loading(&path)
                && !self.cache_manager.is_pending_upload(&path)
                && !self.cache_manager.is_failed(&path)
            {
                // Mark as loading here because selection-triggered requests bypass item slot code.
                self.cache_manager.loading_set.insert(path.clone());
                self.request_thumbnail_load_with_modified(path.clone(), 512, modified);
            }

            let active_tab_id = self.tab_manager.active().id;

            // SPECIAL CASE: GIF Autoplay logic
            let is_gif = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.to_lowercase() == "gif")
                .unwrap_or(false);

            if is_gif {
                // Initialize async GIF player
                use crate::ui::components::media_preview::GifPlayer;
                let data = self.gif_manager.request_load(&path);
                self.selected_gif = Some(GifPlayer::new(path.clone(), data));
            } else {
                // Not a GIF -> Cleanup non-active GIFs (subject to memory/TTL)
                self.gif_manager.cleanup(false);

                // CLEANUP LOGIC: If we are the owner of a VIDEO, and focus changed to a DIFFERENT file, stop the player.
                let is_owner = self.media_preview_owner_tab_id == Some(active_tab_id);
                if is_owner {
                    use crate::ui::components::media_preview::MediaPreview;
                    let should_stop = match &mut self.media_preview {
                        Some(MediaPreview::Video(player)) => player.path != path,
                        _ => false, // GIFs/Images don't "own" global media_preview anymore
                    };

                    if should_stop {
                        self.destroy_media_preview();
                    }
                }
            }
        } else {
            // No selection -> if owner, clear media
            // Also cleanup ALL GIFs immediately as there's no active preview
            self.gif_manager.cleanup(true);

            let active_tab_id = self.tab_manager.active().id;
            if self.media_preview_owner_tab_id == Some(active_tab_id) {
                self.destroy_media_preview();
            }
        }

        // CRITICAL: Sync visibility whenever selection changes
        self.update_video_visibility();
    }

    /// Selects a file/folder by full path in the current `items` snapshot.
    /// Returns `true` when found and selected.
    pub fn select_item_by_path(&mut self, target_path: &std::path::Path) -> bool {
        let target = self.items.iter().enumerate().find_map(|(idx, item)| {
            if item.path == target_path {
                Some((idx, item.clone()))
            } else {
                None
            }
        });

        let Some((idx, item)) = target else {
            return false;
        };

        self.selected_item = Some(idx);
        self.selected_file = Some(item.clone());
        self.multi_selection.clear();
        self.multi_selection.insert(item.path.clone());
        self.selection_anchor = Some(idx);
        self.scroll_to_selected = true;
        self.update_selected_thumbnail();

        true
    }

    /// Limpa a seleção atual, o thumbnail persistente, metadados e a busca.
    /// Útil durante navegação entre pastas.
    /// NOTE: Only clears media_preview if current tab is the owner.
    pub fn reset_selection_and_search(&mut self) {
        // Selection change only updates the persistent thumbnail and metadata.
        // It NO LONGER clears the global media_preview.

        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        self.selected_metadata = None;
        self.search_query.clear();
        self.multi_selection.clear();
        self.context_menu.target_paths.clear();
        self.renaming_state = None;
        self.selected_gif = None;
        self.scroll_offset_y = 0.0;

        // Reset also drops all active GIF previews
        self.gif_manager.cleanup(true);

        // CLEANUP LOGIC: If owner resets selection, clear media
        let active_tab_id = self.tab_manager.active().id;
        if self.media_preview_owner_tab_id == Some(active_tab_id) {
            self.destroy_media_preview();
        }

        // CRITICAL: Sync visibility whenever selection is reset
        self.update_video_visibility();
    }

    /// Control player visibility based on current tab ownership.
    /// Shows video only when current tab is the owner, hides otherwise.
    /// Audio continues playing when hidden (video only hidden visually).
    /// This NEVER pauses, stops, or clears the media - just visual hide/show.
    pub fn update_video_visibility(&mut self) {
        if let Some(crate::ui::components::media_preview::MediaPreview::Video(player)) =
            &mut self.media_preview
        {
            let active_tab_id = self.tab_manager.active().id;
            let is_owner = self.media_preview_owner_tab_id == Some(active_tab_id);

            // Should be visible ONLY if:
            // 1. Current tab is the owner
            // 2. Preview panel is showing
            // 3. Selection matches the video path
            let selected_path_matches = self
                .selected_file
                .as_ref()
                .is_some_and(|f| f.path == player.path);

            let visible = is_owner && self.show_preview_panel && selected_path_matches;

            player.set_visibility(visible);
        }
    }
}
