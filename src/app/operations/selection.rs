//! Selection state management
//!
//! This module handles updates to the selected item, including thumbnail syncing and clearing selection state.
//!
//! IMPORTANT: Media preview has owner-based protection. Only the owner tab can modify playback state.
//! Non-owner tabs can change their own selection without affecting the global media player.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::diagnostic_logger::{diag_info, field_u64};
use std::path::Path;

enum SelectedPreviewOverlayAction {
    None,
    BlockedInArchive,
    PlayMedia(std::path::PathBuf),
    OpenPdf(std::path::PathBuf),
    OpenImage(std::path::PathBuf),
    OpenText(std::path::PathBuf),
}

fn normalize_selection_path(path: &Path) -> String {
    let path = path.to_string_lossy().to_lowercase();
    path.strip_prefix(r"\\?\")
        .unwrap_or(path.as_ref())
        .to_string()
}

fn selected_paths_match(left: &Path, right: &Path) -> bool {
    left == right || normalize_selection_path(left) == normalize_selection_path(right)
}

fn selected_entry_content_changed(old: &FileEntry, new: &FileEntry) -> bool {
    old.is_dir != new.is_dir
        || old.size != new.size
        || old.modified != new.modified
        || old.created != new.created
}

fn selected_entry_display_changed(old: &FileEntry, new: &FileEntry) -> bool {
    selected_entry_content_changed(old, new)
        || old.name != new.name
        || old.folder_cover != new.folder_cover
        || old.sync_status != new.sync_status
        || old.is_hidden != new.is_hidden
}

impl ImageViewerApp {
    pub(crate) fn invalidate_changed_path_preview_state(&mut self, path: &Path) {
        let path = path.to_path_buf();
        let was_loading = self.cache_manager.loading_set.contains(&path);
        let was_pending_upload = self.cache_manager.pending_upload_set.contains(&path);
        let had_pending_thumbnail = self
            .pending_thumbnails
            .iter()
            .any(|thumb| thumb.path == path);
        let queued_removed = self
            .thumbnail_queue
            .remove_paths(std::slice::from_ref(&path));

        self.cache_manager.texture_cache.pop(&path);
        self.cache_manager.pop_rgba_data(&path);
        self.cache_manager.failed_thumbnails.pop(&path);
        self.cache_manager.forget_attempted_thumbnail_bucket(&path);
        self.cache_manager.loading_set.remove(&path);
        self.cache_manager.finish_pending_upload(&path);
        self.pending_thumbnails.retain(|thumb| thumb.path != path);
        crate::workers::thumbnail::clear_failure_cache(&path);

        self.metadata_cache.pop(&path);
        self.metadata_loading.remove(&path);
        self.live_file_size_cache.pop(&path);
        self.live_file_size_loading.remove(&path);

        if self.last_metadata_path.as_ref() == Some(&path) {
            self.last_metadata_path = None;
        }
        if matches!(self.selected_metadata.as_ref(), Some((p, _)) if *p == path) {
            self.selected_metadata = None;
        }
        if self
            .selected_file
            .as_ref()
            .is_some_and(|selected| selected_paths_match(&selected.path, &path))
        {
            self.selected_thumbnail = None;
            self.selected_gif = None;
            self.gif_manager.unload_all();
        }
        if self.last_file_hash_selection.as_ref() == Some(&path) {
            self.selected_file_hash = None;
            self.file_hash_loading.remove(&path);
        }

        if was_loading || was_pending_upload || had_pending_thumbnail || queued_removed > 0 {
            self.bump_thumbnail_request_epoch(&path);
        }
    }

    fn replace_selected_file_with_fresh_entry(&mut self, fresh_entry: FileEntry) -> bool {
        let Some(current) = self.selected_file.as_ref() else {
            return false;
        };
        if !selected_paths_match(&current.path, &fresh_entry.path) {
            return false;
        }

        let content_changed = selected_entry_content_changed(current, &fresh_entry);
        let display_changed = selected_entry_display_changed(current, &fresh_entry);
        if !display_changed {
            return false;
        }

        let fresh_path = fresh_entry.path.clone();
        if content_changed {
            self.invalidate_changed_path_preview_state(&fresh_path);
            if !fresh_entry.is_dir {
                self.enqueue_disk_cache_invalidations_forced(vec![fresh_path]);
            }
        }

        self.selected_file = Some(fresh_entry);
        self.update_video_visibility();
        self.ui_ctx.request_repaint();
        true
    }

    pub(crate) fn sync_selected_file_from_all_items(&mut self) -> bool {
        let Some(selected_path) = self.selected_file.as_ref().map(|file| file.path.clone()) else {
            return false;
        };

        let fresh_entry = self
            .all_items
            .iter()
            .find(|item| selected_paths_match(&item.path, &selected_path))
            .cloned();

        fresh_entry
            .map(|entry| self.replace_selected_file_with_fresh_entry(entry))
            .unwrap_or(false)
    }

    pub fn ensure_detail_panel_thumbnail_for_file(&mut self, file: &FileEntry) {
        self.ensure_detail_panel_thumbnail_request(
            file.path.clone(),
            file.modified,
            file.is_media(),
            crate::domain::thumbnail::detail_preview_size(&file.path),
        );
    }

    fn ensure_detail_panel_thumbnail_request(
        &mut self,
        path: std::path::PathBuf,
        modified: u64,
        is_media: bool,
        size: u32,
    ) {
        if !is_media || self.cache_manager.is_failed(&path) {
            return;
        }

        let effective_req_size = self.effective_thumbnail_request_size_px(size);
        let required_preview_bucket =
            crate::workers::thumbnail::processing::get_bucket_size(effective_req_size);

        let attempted_bucket = self.cache_manager.attempted_thumbnail_bucket_for(&path);
        // True when we've already requested at the detail panel's required quality.
        // Some media files (notably videos) cannot produce thumbnails at higher
        // resolutions than their native frame size.  Once we've attempted the
        // top bucket, whatever is in the texture cache is the best available.
        let already_attempted_max_quality =
            attempted_bucket.is_some_and(|bucket| bucket >= required_preview_bucket);

        let tex_in_cache = self.cache_manager.texture_cache.peek(&path);
        let has_required_texture = tex_in_cache.as_ref().is_some_and(|tex| {
            let tex_size = tex.size();
            (tex_size[0].max(tex_size[1]) as u32) >= size
        });

        // Best-effort promotion: when we've already attempted at the required
        // quality bucket and the result is smaller than ideal, accept it as the
        // best available rather than falling back to a generic icon.
        let request_in_flight =
            self.cache_manager.is_loading(&path) || self.cache_manager.is_pending_upload(&path);
        let promote_best_effort = already_attempted_max_quality && !request_in_flight;

        if let Some(tex) = tex_in_cache {
            if self
                .selected_file
                .as_ref()
                .is_some_and(|selected| selected.path == path)
                && (has_required_texture || promote_best_effort)
            {
                self.selected_thumbnail = Some(tex.clone());
            }
        }

        if !has_required_texture {
            if request_in_flight {
                // A request is already in flight. If it is still queued, move
                // the selected file ahead of list/grid prefetch work and
                // upgrade it to the detail-panel size.
                if self.cache_manager.is_loading(&path)
                    && !self.cache_manager.is_pending_upload(&path)
                {
                    let effective_gen = if self.use_active_generation_for_thumbnail_requests {
                        self.current_generation
                            .load(std::sync::atomic::Ordering::Relaxed)
                    } else {
                        self.generation
                    };
                    let request_epoch = self
                        .thumbnail_request_epochs
                        .get(&path)
                        .copied()
                        .unwrap_or(0);

                    if self.thumbnail_queue.promote_pending_to_interactive(
                        &path,
                        effective_gen,
                        effective_req_size,
                        0,
                        modified,
                        request_epoch,
                    ) {
                        self.cache_manager
                            .note_attempted_thumbnail_bucket(&path, required_preview_bucket);
                    }
                }
            } else if already_attempted_max_quality {
                // Already tried at max quality; the best available texture is
                // in cache.  No further requests needed.
                if !self.cache_manager.best_effort_notified.contains(&path) {
                    self.cache_manager.best_effort_notified.insert(path.clone());
                    diag_info(
                        "preview_thumbnail",
                        "best_effort_accepted",
                        &[
                            field_u64("logical_req_size", size as u64),
                            field_u64("effective_req_size", effective_req_size as u64),
                            field_u64("attempted_bucket", attempted_bucket.unwrap_or(0) as u64),
                            field_u64("required_bucket", required_preview_bucket as u64),
                        ],
                    );
                }
            } else {
                self.cache_manager.loading_set.insert(path.clone());
                self.request_thumbnail_load_with_index_and_modified(path, size, 0, modified);
            }
        }
    }

    /// Keeps the visible focus index aligned with the currently selected file.
    ///
    /// When filtering or sorting changes the current `items` snapshot, the old
    /// numeric `selected_item` index may now point to a different row/tile. The
    /// preview panel should continue following `selected_file`, but the list/grid
    /// focus ring must only appear if that same file is still visible.
    pub fn reconcile_visible_selection_index(&mut self) {
        let resolved_index = self.selected_file.as_ref().and_then(|selected| {
            self.items
                .iter()
                .position(|item| selected_paths_match(&item.path, &selected.path))
        });

        if let Some(index) = resolved_index {
            if let Some(fresh_entry) = self.items.get(index).cloned() {
                self.replace_selected_file_with_fresh_entry(fresh_entry);
            }
        }

        if self.selected_item == resolved_index {
            return;
        }

        self.selected_item = resolved_index;

        if self.multi_selection.len() <= 1 {
            self.selection_anchor = resolved_index;
        }

        if resolved_index.is_none() {
            self.scroll_to_selected = false;
            self.scroll_request = crate::app::state::ScrollRequest::None;

            // When the selected file is not in the visible items but we have
            // data (all_items non-empty), the file was filtered out by search.
            // Clear the selection so the preview panel shows "no selection"
            // instead of stale data from the previously selected file.
            if self.selected_file.is_some() && !self.all_items.is_empty() {
                self.selected_file = None;
                self.selected_metadata = None;
                self.multi_selection.clear();
                self.update_selected_thumbnail();
            }
        }
    }

    /// Kill the standalone video player process if one is running.
    /// Reloads volume from the database because the standalone player persists
    /// volume changes immediately on each adjustment.
    pub fn kill_video_player_process(&mut self) {
        if let Some(mut child) = self.video_player_process.take() {
            log::debug!("[VIDEO-PLAYER] Killing standalone video player process");
            let _ = child.kill();
            // Don't block on child.wait() — TerminateProcess is immediate on
            // Windows and process::exit will reap any zombies.

            // The standalone player saves volume to DB on every change,
            // so reload now so the next video uses the latest volume.
            if let Some(vol_str) = self.app_state_db.get_preference("media_volume") {
                if let Ok(vol) = vol_str.parse::<f32>() {
                    self.session_volume = vol.clamp(0.0, 1.0);
                }
            }
        }
    }

    /// Reap the standalone video player process if it has exited naturally.
    /// Called periodically from the update loop to detect when the user closes the player window.
    /// When the player exits, reloads volume from the database so that volume changes
    /// made in the standalone player are reflected in the main app.
    pub fn reap_video_player_process(&mut self) {
        if let Some(child) = &mut self.video_player_process {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    log::debug!("[VIDEO-PLAYER] Standalone player exited");
                    self.video_player_process = None;

                    // The standalone player persists volume changes to the database.
                    // Reload session_volume so the next video opens at the correct level.
                    if let Some(vol_str) = self.app_state_db.get_preference("media_volume") {
                        if let Ok(vol) = vol_str.parse::<f32>() {
                            self.session_volume = vol.clamp(0.0, 1.0);
                        }
                    }
                }
                Ok(None) => {} // Still running
                Err(e) => {
                    log::warn!("[VIDEO-PLAYER] Error checking player status: {}", e);
                    self.video_player_process = None;
                }
            }
        }
    }

    /// Teardown media preview resources immediately (MPV buffers, thread, HWND).
    pub fn destroy_media_preview(&mut self) {
        if let Some(mut preview) = self.media_preview.take() {
            preview.shutdown();
        }
        self.media_preview_owner_tab_id = None;
        self.ui_ctx.request_repaint();
        // Memory maintenance is handled by the periodic 2s check in the update loop.
        // Running it synchronously here caused ~880ms UI stalls: MPV shutdown releases
        // large decoder buffers, and the immediate cache trim amplifies OS working-set
        // pressure, causing page faults on the very next render frame.
    }

    /// Starts media playback in the preview panel using the same flow as clicking
    /// the play overlay in the details panel.
    pub fn request_video_preview_playback(&mut self, path: std::path::PathBuf) {
        use crate::ui::components::media_preview::MediaPreview;
        use crate::ui::components::MpvPreview;

        // Kill standalone video player process if one is running
        self.kill_video_player_process();

        // TAKE OVER: Stop and drop existing player if any
        if matches!(self.media_preview.as_ref(), Some(MediaPreview::Video(_))) {
            self.destroy_media_preview();
        }

        // Take ownership and start new player
        let mut player = MpvPreview::new(path);
        player.play_on_init = true; // Start playing as soon as initialized
        player.show_player = true; // Ensure player is visible immediately

        // Set initial volume (will be applied when MPV is ready)
        player.initial_volume = self.session_volume;

        let tab_id = self.tab_manager.active().id;
        self.media_preview = Some(MediaPreview::Video(Box::new(player)));
        self.media_preview_owner_tab_id = Some(tab_id);

        // Final sync: hide/show correctly
        self.update_video_visibility();
    }

    fn selected_standalone_media_path(&self) -> Option<std::path::PathBuf> {
        let selected = self.selected_file.as_ref()?;
        if selected.is_dir {
            return None;
        }

        let ext = selected
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_owned();

        let is_video = crate::infrastructure::windows::is_video_extension(&ext);
        let is_audio = crate::infrastructure::windows::is_audio_extension(&ext);
        if !is_video && !is_audio {
            return None;
        }

        let path = selected.path.clone();
        if crate::domain::file_entry::is_path_inside_archive(&path) {
            return None;
        }

        Some(path)
    }

    pub(crate) fn should_show_secondary_toolbar_media_play_button(&self) -> bool {
        self.selected_standalone_media_path().is_some()
    }

    pub fn open_selected_media_in_standalone_player(&mut self) -> bool {
        use crate::ui::components::media_preview::MediaPreview;

        let Some(path) = self.selected_standalone_media_path() else {
            return false;
        };

        self.kill_video_player_process();

        if matches!(self.media_preview.as_ref(), Some(MediaPreview::Video(_))) {
            self.destroy_media_preview();
        }

        if let Some(child) = crate::video_player::open_video_player(path, 0.0, self.session_volume)
        {
            self.video_player_process = Some(child);
            true
        } else {
            false
        }
    }

    fn selected_preview_overlay_action(&self) -> SelectedPreviewOverlayAction {
        let Some(selected) = self.selected_file.as_ref() else {
            return SelectedPreviewOverlayAction::None;
        };

        if selected.is_dir {
            return SelectedPreviewOverlayAction::None;
        }

        let ext = selected
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_owned();
        let is_video = crate::infrastructure::windows::is_video_extension(&ext);
        let is_audio = crate::infrastructure::windows::is_audio_extension(&ext);
        let is_pdf = ext.eq_ignore_ascii_case("pdf");
        let is_image = crate::infrastructure::windows::is_image_extension(&ext);
        let is_text = crate::text_viewer::is_text_extension(&ext);

        if !is_video && !is_audio && !is_pdf && !is_image && !is_text {
            return SelectedPreviewOverlayAction::None;
        }

        let path = selected.path.clone();
        if crate::domain::file_entry::is_path_inside_archive(&path) {
            return SelectedPreviewOverlayAction::BlockedInArchive;
        }

        if is_video || is_audio {
            SelectedPreviewOverlayAction::PlayMedia(path)
        } else if is_pdf {
            SelectedPreviewOverlayAction::OpenPdf(path)
        } else if is_image {
            SelectedPreviewOverlayAction::OpenImage(path)
        } else {
            SelectedPreviewOverlayAction::OpenText(path)
        }
    }

    pub(crate) fn should_consume_space_for_selected_preview_overlay_action(&self) -> bool {
        !matches!(
            self.selected_preview_overlay_action(),
            SelectedPreviewOverlayAction::None
        )
    }

    /// Triggers the same action exposed by the preview overlays
    /// (video play / image viewer / PDF viewer) for the currently selected file.
    /// Returns true when an action was triggered.
    pub fn trigger_selected_preview_overlay_action(&mut self) -> bool {
        match self.selected_preview_overlay_action() {
            SelectedPreviewOverlayAction::PlayMedia(path) => {
                self.request_video_preview_playback(path);
                true
            }
            SelectedPreviewOverlayAction::OpenPdf(path) => {
                crate::pdf_viewer::open_pdf_viewer(path);
                true
            }
            SelectedPreviewOverlayAction::OpenImage(path) => {
                crate::image_viewer::open_image_viewer(path);
                true
            }
            SelectedPreviewOverlayAction::OpenText(path) => {
                crate::text_viewer::open_text_viewer(path);
                true
            }
            SelectedPreviewOverlayAction::BlockedInArchive | SelectedPreviewOverlayAction::None => {
                false
            }
        }
    }

    pub fn update_selected_thumbnail(&mut self) {
        // Selection change only updates the persistent thumbnail and metadata.
        // It NO LONGER clears or sets the global media_preview automatically.
        // This allows playback to continue in the background while the user browses.

        self.selected_thumbnail = None;
        self.selected_gif = None;

        if self.selected_file.is_some() {
            self.defer_preview_work_after_selection = true;
            self.update_video_visibility();
            self.ui_ctx.request_repaint();
            return;
        }

        self.defer_preview_work_after_selection = false;

        // No selection -> if owner, clear media.
        self.gif_manager.unload_all();

        let active_tab_id = self.tab_manager.active().id;
        if self.media_preview_owner_tab_id == Some(active_tab_id) {
            self.destroy_media_preview();
        }

        // CRITICAL: Sync visibility whenever selection changes
        self.update_video_visibility();
    }

    pub fn needs_selected_preview_preparation(&self) -> bool {
        self.selected_file.is_some()
            && self.selected_thumbnail.is_none()
            && self.selected_gif.is_none()
            && !self.defer_preview_work_after_selection
    }

    pub fn prepare_selected_preview_for_file(&mut self, file: &FileEntry) {
        if self
            .selected_file
            .as_ref()
            .is_none_or(|selected| selected.path != file.path)
        {
            return;
        }

        let path = file.path.clone();
        let thumbnail_size = crate::domain::thumbnail::detail_preview_size(&path);
        self.ensure_detail_panel_thumbnail_request(
            path.clone(),
            file.modified,
            file.is_media(),
            thumbnail_size,
        );

        let active_tab_id = self.tab_manager.active().id;
        let is_owner = self.media_preview_owner_tab_id == Some(active_tab_id);
        if is_owner {
            use crate::ui::components::media_preview::MediaPreview;
            let should_stop = match &mut self.media_preview {
                Some(MediaPreview::Video(player)) => player.path != path,
                _ => false,
            };

            if should_stop {
                self.destroy_media_preview();
            }
        }

        if crate::domain::thumbnail::detail_preview_size(&path)
            == crate::domain::thumbnail::DETAIL_PREVIEW_GIF_SIZE
        {
            use crate::ui::components::media_preview::GifPlayer;
            self.gif_manager.unload_except(Some(&path));
            let needs_player = self
                .selected_gif
                .as_ref()
                .is_none_or(|player| player.path != path);
            if needs_player {
                let data = self.gif_manager.request_load(&path);
                self.selected_gif = Some(GifPlayer::new(path.clone(), data));
            }
        } else {
            self.selected_gif = None;
            self.gif_manager.unload_all();
        }

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

    /// Selects every item in the current loaded folder/item snapshot.
    pub fn select_all_current_items(&mut self) -> bool {
        if self.items.is_empty() {
            return false;
        }

        let previous_selected_path = self.selected_file.as_ref().map(|item| item.path.clone());
        self.multi_selection.clear();
        self.multi_selection.reserve(self.items.len());

        for item in self.items.iter() {
            self.multi_selection.insert(item.path.clone());
        }

        let focus_index = self
            .selected_file
            .as_ref()
            .and_then(|selected| {
                self.items
                    .iter()
                    .position(|item| item.path == selected.path)
            })
            .or_else(|| self.selected_item.filter(|idx| *idx < self.items.len()))
            .unwrap_or(0);
        let focused_item = self.items[focus_index].clone();
        let focused_path = focused_item.path.clone();

        self.selected_item = Some(focus_index);
        self.selected_file = Some(focused_item);
        self.selection_anchor = Some(focus_index);
        self.rectangle_selection_state = None;

        if previous_selected_path.as_ref() != Some(&focused_path) {
            self.update_selected_thumbnail();
        }

        true
    }

    /// Clears the current selection, persistent thumbnail, metadata and search.
    /// Useful during navigation between folders.
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
        self.scroll_offset_x = 0.0;

        // Reset also drops all active GIF previews
        self.gif_manager.unload_all();

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
