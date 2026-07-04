use crate::app::state::ImageViewerApp;
use crate::workers::file_operation_worker::FileOperationResult;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn process_file_operation_results(
        &mut self,
        current_path_norm: &str,
        ctx: &eframe::egui::Context,
    ) {
        const MAX_FILE_OP_RESULTS_PER_FRAME: usize = 96;
        let budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(2)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(3)
        } else {
            Duration::from_millis(5)
        };

        let start = Instant::now();
        let mut processed = 0usize;
        let mut has_more = false;

        while processed < MAX_FILE_OP_RESULTS_PER_FRAME {
            if start.elapsed() >= budget {
                has_more = true;
                break;
            }

            match self.file_operation_state.file_op_res_receiver.try_recv() {
                Ok(res) => {
                    processed += 1;
                    match res {
                        FileOperationResult::RenameCompleted {
                            path,
                            new_name,
                            parent_folder,
                        } => self.handle_rename_completed(
                            path,
                            new_name,
                            parent_folder,
                            current_path_norm,
                        ),
                        FileOperationResult::RenameBatchProgress {
                            completed,
                            total,
                            current_name,
                        } => self.handle_rename_batch_progress(completed, total, current_name),
                        FileOperationResult::RenameBatchCompleted { count } => {
                            self.handle_rename_batch_completed(count)
                        }
                        FileOperationResult::DriveRenameCompleted {
                            drive_path,
                            new_label,
                        } => self.handle_drive_rename_completed(drive_path, new_label),
                        FileOperationResult::DriveRenameFailed {
                            drive_path,
                            error,
                            cancelled,
                        } => self.handle_drive_rename_failed(drive_path, error, cancelled),
                        FileOperationResult::OperationFailed { message } => {
                            self.notifications.warning(message);
                            self.restore_app_focus();
                        }
                        FileOperationResult::RecycleBinChanged => self.handle_recycle_bin_changed(),
                        FileOperationResult::RestoreCompleted { parent_folders } => {
                            self.handle_parent_folder_updates(parent_folders, current_path_norm)
                        }
                        FileOperationResult::DeleteCompleted {
                            parent_folders,
                            deleted_paths,
                        } => {
                            self.handle_delete_completed(
                                parent_folders,
                                deleted_paths,
                                current_path_norm,
                            );
                            self.cleanup_deleted_pinned_folders();
                        }
                        FileOperationResult::CopyCompleted {
                            dest_folder,
                            copied_dests,
                        } => {
                            self.handle_copy_completed(dest_folder, copied_dests, current_path_norm)
                        }
                        FileOperationResult::MoveCompleted {
                            source_folder,
                            dest_folder,
                            source_path,
                            moved_dest,
                        } => {
                            self.handle_move_completed(
                                source_folder,
                                dest_folder,
                                source_path,
                                moved_dest,
                                current_path_norm,
                            );
                            self.cleanup_deleted_pinned_folders();
                        }
                        FileOperationResult::MoveBatchCompleted {
                            source_folders,
                            dest_folder,
                            moved_files,
                            known_moved_pairs,
                        } => {
                            self.handle_move_batch_completed(
                                source_folders,
                                dest_folder,
                                moved_files,
                                known_moved_pairs,
                                current_path_norm,
                            );
                            self.cleanup_deleted_pinned_folders();
                        }
                        FileOperationResult::Finished => self.handle_file_operation_finished(true),
                        FileOperationResult::FinishedNoRefresh => {
                            self.handle_file_operation_finished(false)
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if processed >= MAX_FILE_OP_RESULTS_PER_FRAME {
            has_more = true;
        }

        if has_more {
            ctx.request_repaint();
        }
    }

    fn invalidate_folder_and_tab_caches(&mut self, folder: &Path) {
        self.invalidate_directory_caches(folder);
        // Keep sidebar folder tree in sync with file operations (delete/rename/move)
        self.sidebar_tree.clear_children(folder);
        let folder_norm = Self::normalize_for_match(folder);
        self.clear_tab_cache_for_normalized_path(&folder_norm);
    }

    fn invalidate_folder_listing_and_tab_caches(&mut self, folder: &Path) {
        self.invalidate_directory_listing_caches(folder);
        self.sidebar_tree.clear_children(folder);
        let folder_norm = Self::normalize_for_match(folder);
        self.clear_tab_cache_for_normalized_path(&folder_norm);
        self.enqueue_disk_cache_invalidations(vec![folder.to_path_buf()]);
        self.schedule_folder_cover_refresh(folder);
    }

    fn update_renamed_item_in_place(
        items: &mut [crate::domain::file_entry::FileEntry],
        old_path: &Path,
        old_path_norm: &str,
        new_path: &Path,
        new_name: &str,
    ) {
        if let Some(item) = items.iter_mut().find(|item| {
            item.path == old_path || Self::normalize_for_match(&item.path) == old_path_norm
        }) {
            item.path = new_path.to_path_buf();
            item.name = new_name.to_string();
        }
    }

    fn apply_rename_completed_to_memory(
        &mut self,
        path: &Path,
        new_name: &str,
        parent_folder: &Path,
    ) -> PathBuf {
        let old_path = path.to_path_buf();
        let new_path = parent_folder.join(new_name);
        let path_str = Self::normalize_for_match(path);

        if let Some(selected) = &mut self.selected_file {
            if Self::normalize_for_match(&selected.path) == path_str {
                selected.path = new_path.clone();
                selected.name = new_name.to_string();
            }
        }

        if self.multi_selection.remove(&old_path) {
            self.multi_selection.insert(new_path.clone());
        }

        let should_destroy_preview = match self.media_preview.as_ref() {
            Some(crate::ui::components::media_preview::MediaPreview::Video(player)) => {
                Self::normalize_for_match(&player.path) == path_str
            }
            _ => false,
        };
        if should_destroy_preview {
            self.destroy_media_preview();
        }

        // Update items in-place so no full folder reload is needed.
        // Prefer exact PathBuf equality and only fall back to normalized matching
        // for verbatim-prefix or casing differences.
        let items_diverged = !Arc::ptr_eq(&self.items, &self.all_items);
        let all_items = Arc::make_mut(&mut self.all_items);
        Self::update_renamed_item_in_place(all_items, &old_path, &path_str, &new_path, new_name);
        if items_diverged {
            let items = Arc::make_mut(&mut self.items);
            Self::update_renamed_item_in_place(items, &old_path, &path_str, &new_path, new_name);
        }

        // Move thumbnail cache entries from old path to new path so existing
        // thumbnails remain visible without needing re-extraction.
        if let Some(texture) = self.cache_manager.texture_cache.pop(&old_path) {
            self.cache_manager
                .texture_cache
                .put(new_path.clone(), texture);
        }
        if let Some((data, w, h)) = self.cache_manager.pop_rgba_data(&old_path) {
            self.cache_manager
                .put_rgba_data(new_path.clone(), data, w, h);
        }
        if let Some(bucket) = self
            .cache_manager
            .attempted_thumbnail_bucket
            .remove(&old_path)
        {
            self.cache_manager
                .attempted_thumbnail_bucket
                .insert(new_path.clone(), bucket);
        }
        self.cache_manager.loading_set.remove(&old_path);
        self.cache_manager.failed_thumbnails.pop(&old_path);
        // Clear any stale failure record under the new path.
        self.cache_manager.failed_thumbnails.pop(&new_path);

        new_path
    }

    fn remap_visual_caches_for_path(&mut self, old_path: &Path, new_path: &Path) {
        if old_path == new_path {
            return;
        }

        let old_path = old_path.to_path_buf();
        let new_path = new_path.to_path_buf();

        if let Some(texture) = self.cache_manager.texture_cache.pop(&old_path) {
            self.cache_manager
                .texture_cache
                .put(new_path.clone(), texture);
        }
        if let Some((data, w, h)) = self.cache_manager.pop_rgba_data(&old_path) {
            self.cache_manager
                .put_rgba_data(new_path.clone(), data, w, h);
        }
        if let Some(bucket) = self
            .cache_manager
            .attempted_thumbnail_bucket
            .remove(&old_path)
        {
            self.cache_manager
                .attempted_thumbnail_bucket
                .insert(new_path.clone(), bucket);
        }

        if let Some(folder_preview) = self.cache_manager.folder_preview_cache.pop(&old_path) {
            self.cache_manager
                .folder_preview_cache
                .put(new_path.clone(), folder_preview);
        }

        let had_pending_upload = self.cache_manager.pending_upload_set.remove(&old_path);
        let mut remapped_pending_upload = false;
        for thumbnail in self.pending_thumbnails.iter_mut() {
            if thumbnail.path == old_path {
                thumbnail.path = new_path.clone();
                remapped_pending_upload = true;
            }
        }
        if had_pending_upload && remapped_pending_upload {
            self.cache_manager
                .pending_upload_set
                .insert(new_path.clone());
        }

        self.thumbnail_queue
            .remove_paths(std::slice::from_ref(&old_path));
        self.cache_manager.loading_set.remove(&old_path);
        self.cache_manager.failed_thumbnails.pop(&old_path);
        self.cache_manager.failed_thumbnails.pop(&new_path);
        self.cache_manager
            .forget_thumbnail_request_cooldown(&old_path);
        self.cache_manager
            .forget_thumbnail_request_cooldown(&new_path);

        self.cache_manager.folder_preview_loading.remove(&old_path);
        self.cache_manager
            .forget_folder_preview_request_cooldown(&old_path);
        self.cache_manager
            .forget_folder_preview_request_cooldown(&new_path);
        if self.pending_folder_preview_replace.remove(&old_path) {
            self.pending_folder_preview_replace.insert(new_path.clone());
        }
        if self
            .suppress_next_folder_preview_invalidation
            .remove(&old_path)
        {
            self.suppress_next_folder_preview_invalidation
                .insert(new_path);
        }
    }

    fn enqueue_thumbnail_cache_renames(&self, moves: &[(PathBuf, PathBuf)]) {
        if moves.is_empty() {
            return;
        }

        use crate::app::init_workers::CacheInvalidationEntry;
        let entries: Vec<_> = moves
            .iter()
            .map(|(source, dest)| CacheInvalidationEntry {
                path: source.clone(),
                force: false,
                rename_to: Some(dest.clone()),
            })
            .collect();
        let _ = self
            .file_operation_state
            .disk_cache_invalidation_sender
            .send(entries);
    }

    fn handle_rename_completed(
        &mut self,
        path: PathBuf,
        new_name: String,
        parent_folder: PathBuf,
        current_path_norm: &str,
    ) {
        let parent_str = Self::normalize_for_match(parent_folder.as_path());
        self.invalidate_folder_and_tab_caches(&parent_folder);
        let new_path =
            self.apply_rename_completed_to_memory(&path, &new_name, parent_folder.as_path());
        self.move_tag_assignments_for_path(&path, &new_path);

        // Migrate the disk-cache row from old path to new path so thumbnails
        // are preserved after the in-memory caches are evicted on scroll.
        // Use a rename entry instead of the old forced-invalidate.
        use crate::app::init_workers::CacheInvalidationEntry;
        let _ = self
            .file_operation_state
            .disk_cache_invalidation_sender
            .send(vec![CacheInvalidationEntry {
                path: path.clone(),
                force: false,
                rename_to: Some(new_path.clone()),
            }]);

        if parent_str == current_path_norm {
            // Clear the dirty flag set by invalidate_folder_and_tab_caches above.
            // The in-place update already reflects the rename in self.items /
            // self.all_items, so the tab-switch staleness check must not force
            // an unnecessary full reload of the already-correct view.
            // Inactive tabs had their cached items cleared by
            // clear_tab_cache_for_normalized_path and will reload via the
            // empty-items path in sync_from_tab instead.
            self.directory_dirty_registry.clear_dirty(&parent_folder);
            // Only move selection for single (non-batch) renames.
            if self.file_operation_state.batch_rename_progress.is_none() {
                self.pending_select_path = Some(new_path);
            }
            self.pending_items_rebuild = true;
        }
    }

    fn handle_rename_batch_progress(
        &mut self,
        completed: usize,
        total: usize,
        current_name: String,
    ) {
        if total == 0 {
            self.file_operation_state.batch_rename_progress = None;
            return;
        }

        self.file_operation_state.batch_rename_progress =
            Some(crate::app::file_operation_state::BatchRenameProgress {
                completed: completed.min(total),
                total,
                current_name: Some(current_name),
            });
    }

    fn handle_rename_batch_completed(&mut self, count: usize) {
        // Each item was already handled incrementally by handle_rename_completed
        // (in-place path/name update, cache key migration, dirty-flag cleanup).
        // This handler only finalises the progress indicator and shows the toast.
        self.file_operation_state.batch_rename_progress = None;
        if count > 0 {
            self.notifications.success(
                rust_i18n::t!("batch_rename.progress_complete", count = count).to_string(),
            );
        }
    }

    fn handle_recycle_bin_changed(&mut self) {
        if self.navigation_state.is_recycle_bin_view {
            #[cfg(debug_assertions)]
            log::debug!("[RECYCLE] Operation finished, refreshing view.");
            self.setup_recycle_bin_view();
            // Keep tab manager synchronized with recycle-bin virtual view.
            self.sync_to_tab();
        }
    }

    fn handle_parent_folder_updates(
        &mut self,
        parent_folders: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        let mut should_reload_current = false;
        for parent in parent_folders {
            self.invalidate_folder_and_tab_caches(&parent);
            let parent_str = Self::normalize_for_match(parent.as_path());
            if parent_str == current_path_norm {
                should_reload_current = true;
            }
        }

        if should_reload_current {
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }
    }

    fn handle_delete_completed(
        &mut self,
        parent_folders: Vec<PathBuf>,
        deleted_paths: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        for path in &deleted_paths {
            self.cache_manager.texture_cache.pop(path);
            self.cache_manager.loading_set.remove(path);
            self.cache_manager.pop_rgba_data(path);
            self.cache_manager.failed_thumbnails.pop(path);
            self.multi_selection.remove(path);
        }

        if let Some(selected) = &self.selected_file {
            if deleted_paths.contains(&selected.path) {
                self.selected_item = None;
                self.selected_file = None;
            }
        }

        self.navigate_inactive_panel_after_deleted_paths(&deleted_paths);
        self.clear_tag_assignments_for_paths(&deleted_paths);
        self.enqueue_disk_cache_invalidations_forced(deleted_paths);
        let affected_parent_folders: Vec<&PathBuf> = parent_folders.iter().collect();
        self.reload_inactive_panel_if_matches(&affected_parent_folders);
        self.handle_parent_folder_updates(parent_folders, current_path_norm);
    }

    pub(super) fn restore_app_focus(&self) {
        if self.layout.saved_is_minimized {
            return;
        }

        if let Some(hwnd) = self.native_hwnd {
            if crate::infrastructure::windows::is_window_minimized(hwnd) {
                return;
            }

            crate::infrastructure::windows::restore_window_foreground(hwnd);
        }
        self.ui_ctx
            .send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
        self.ui_ctx.request_repaint();
    }

    fn handle_drive_rename_completed(&mut self, drive_path: PathBuf, new_label: String) {
        let drive_path_str = drive_path.to_string_lossy().to_string();
        let display_name =
            crate::infrastructure::windows::format_drive_display_name(&drive_path_str, &new_label);
        if let Some((_, label)) = self
            .drive_state
            .disks
            .iter_mut()
            .find(|(path, _)| *path == drive_path_str)
        {
            *label = display_name.clone();
        }
        self.drive_state.remove_cached_drive_info(&drive_path_str);

        if self.navigation_state.is_computer_view {
            self.setup_computer_view();
            let _ = self.select_item_by_path(&drive_path);
            self.sync_to_tab();
        }

        self.notifications.success(rust_i18n::t!(
            "operations.rename_drive_success",
            drive = drive_path_str,
            name = if new_label.is_empty() {
                rust_i18n::t!("drive_types.default_label").to_string()
            } else {
                new_label
            }
        ));
        self.restore_app_focus();
    }

    fn handle_drive_rename_failed(&mut self, drive_path: PathBuf, error: String, cancelled: bool) {
        let drive_path_str = drive_path.to_string_lossy().to_string();
        if cancelled {
            self.notifications.warning(rust_i18n::t!(
                "operations.rename_drive_cancelled",
                drive = drive_path_str
            ));
            self.restore_app_focus();
            return;
        }

        self.notifications.error(rust_i18n::t!(
            "operations.rename_drive_error",
            drive = drive_path_str,
            error = error
        ));
        self.restore_app_focus();
    }

    fn handle_copy_completed(
        &mut self,
        dest_folder: PathBuf,
        copied_dests: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        let dest_str = Self::normalize_for_match(dest_folder.as_path());
        self.invalidate_folder_listing_and_tab_caches(&dest_folder);

        // Retry files that previously failed thumbnail extraction while copy was in progress.
        self.cache_manager.clear_failed();
        crate::workers::thumbnail::clear_all_failures();

        // Clear write-activity markers for the files/folders we just copied so the
        // stability guard does not delay thumbnail generation after Shell completes.
        // These paths were fully written by Windows Shell — they are safe to read.
        if !copied_dests.is_empty() {
            crate::infrastructure::windows::file_flags::clear_write_activity_after_completed_file_operation(
                &copied_dests,
            );
            self.clear_tag_assignments_for_copied_paths(&copied_dests);
        }

        if dest_str == current_path_norm {
            #[cfg(debug_assertions)]
            log::debug!(
                "[COPY] Dest folder matches current view, reloading: {}",
                self.navigation_state.current_path
            );
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }

        // Reload inactive dual panel if it shows the destination folder
        self.reload_inactive_panel_if_matches(&[&dest_folder]);

        // Suppress watcher-triggered reloads for 2 seconds to avoid redundant
        // reloads from filesystem events generated by the Shell copy operation.
        self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
    }

    fn handle_move_completed(
        &mut self,
        source_folder: PathBuf,
        dest_folder: PathBuf,
        source_path: PathBuf,
        moved_dest: Option<PathBuf>,
        current_path_norm: &str,
    ) {
        let source_str = Self::normalize_for_match(source_folder.as_path());
        let dest_str = Self::normalize_for_match(dest_folder.as_path());

        self.invalidate_folder_listing_and_tab_caches(&source_folder);
        self.invalidate_folder_listing_and_tab_caches(&dest_folder);

        self.cache_manager.clear_failed();
        crate::workers::thumbnail::clear_all_failures();

        // Clear write-activity markers for the moved file so thumbnail generation
        // is not delayed by the stability guard.
        if let Some(dest_path) = moved_dest.as_ref() {
            crate::infrastructure::windows::file_flags::clear_write_activity_after_completed_file_operation(
                std::slice::from_ref(dest_path),
            );
            self.remap_visual_caches_for_path(&source_path, dest_path);
            self.move_tag_assignments_for_path(&source_path, dest_path);
            self.enqueue_thumbnail_cache_renames(&[(source_path.clone(), dest_path.clone())]);
            self.reload_visible_tag_views();
        }

        if current_path_norm == source_str {
            #[cfg(debug_assertions)]
            log::debug!(
                "[MOVE] Source folder matches current view, reloading: {}",
                self.navigation_state.current_path
            );
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }

        if current_path_norm == dest_str {
            #[cfg(debug_assertions)]
            log::debug!(
                "[MOVE] Dest folder matches current view, reloading: {}",
                self.navigation_state.current_path
            );
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }

        // Reload inactive dual panel if it shows source or destination
        self.reload_inactive_panel_if_matches(&[&source_folder, &dest_folder]);

        // Suppress watcher-triggered reloads for 2 seconds to avoid redundant
        // reloads from filesystem events generated by the Shell move operation.
        self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
    }

    fn handle_move_batch_completed(
        &mut self,
        source_folders: Vec<PathBuf>,
        dest_folder: PathBuf,
        moved_files: Vec<PathBuf>,
        known_moved_pairs: Vec<(PathBuf, PathBuf)>,
        current_path_norm: &str,
    ) {
        let dest_str = Self::normalize_for_match(dest_folder.as_path());

        self.cache_manager.clear_failed();
        crate::workers::thumbnail::clear_all_failures();

        let moved_dests: Vec<std::path::PathBuf> = known_moved_pairs
            .iter()
            .map(|(_, dest)| dest.clone())
            .collect();
        if !moved_dests.is_empty() {
            crate::infrastructure::windows::file_flags::clear_write_activity_after_completed_file_operation(
                &moved_dests,
            );
        }
        for (source, dest) in &known_moved_pairs {
            self.remap_visual_caches_for_path(source, dest);
        }
        for (source, dest) in &known_moved_pairs {
            self.move_tag_assignments_for_path(source, dest);
        }
        self.enqueue_thumbnail_cache_renames(&known_moved_pairs);
        if !known_moved_pairs.is_empty() {
            self.reload_visible_tag_views();
        }

        for source_folder in &source_folders {
            self.invalidate_folder_listing_and_tab_caches(source_folder);
        }
        self.invalidate_folder_listing_and_tab_caches(&dest_folder);

        // If any moved file was a folder cover, force re-discovery.
        for source_folder in &source_folders {
            let covers = self
                .app_state_db
                .get_folder_covers(std::slice::from_ref(source_folder));
            if let Some(current_cover) = covers.get(source_folder) {
                if moved_files.iter().any(|f| f == current_cover) {
                    self.app_state_db.remove_folder_cover(source_folder);
                    self.cache_manager.folder_preview_cache.pop(source_folder);
                    let _ = self.cover_worker_sender.send(source_folder.clone());
                    #[cfg(debug_assertions)]
                    log::debug!(
                        "[MOVE-BATCH] Moved file was folder cover for {:?}, requesting recalculation",
                        source_folder
                    );
                }
            }
        }

        let mut should_reload_source_view = false;
        for source_folder in &source_folders {
            let source_str = Self::normalize_for_match(source_folder.as_path());
            if current_path_norm == source_str {
                should_reload_source_view = true;
            }
        }

        if should_reload_source_view {
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }

        if current_path_norm == dest_str {
            #[cfg(debug_assertions)]
            log::debug!(
                "[MOVE-BATCH] Dest folder matches current view, reloading: {}",
                self.navigation_state.current_path
            );
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }

        // Reload inactive dual panel if it shows any affected folder
        let mut affected: Vec<&PathBuf> = source_folders.iter().collect();
        affected.push(&dest_folder);
        self.reload_inactive_panel_if_matches(&affected);

        // Suppress watcher-triggered reloads for 2 seconds to avoid redundant
        // reloads from filesystem events generated by the Shell move operation.
        self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
    }

    fn handle_file_operation_finished(&mut self, refresh_current_view: bool) {
        self.file_operation_state.file_ops_in_progress = self
            .file_operation_state
            .file_ops_in_progress
            .saturating_sub(1);
        log::info!(
            "[FILE-OP] handle_file_operation_finished: ops_remaining={}",
            self.file_operation_state.file_ops_in_progress
        );
        if self.file_operation_state.file_ops_in_progress == 0 {
            if self
                .file_operation_state
                .batch_rename_progress
                .as_ref()
                .is_some_and(|progress| progress.completed >= progress.total)
            {
                self.file_operation_state.batch_rename_progress = None;
            }
            self.pending_auto_reload = false;
            // Keep pending_deletions until folder load completion to avoid stale thumbnail retries.
            if !self.is_loading_folder {
                self.file_operation_state.pending_deletions.clear();
            }

            // Shell dialogs (copy/move/delete/rename) steal focus from the app
            // window (especially via the proxy HWND). Restore it when the last
            // operation in the batch finishes.
            self.restore_app_focus();

            if !refresh_current_view {
                self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
                return;
            }

            // Watcher events were drained but not processed while ops were active.
            // Force a full reload so the view reflects the final state of the folder.
            if !crate::domain::special_paths::is_virtual_path(&self.navigation_state.current_path) {
                if self.is_loading_folder {
                    self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
                    return;
                }

                let current = PathBuf::from(&self.navigation_state.current_path);
                log::info!("[FILE-OP] Invalidating cache for current={:?}", current);
                self.directory_dirty_registry.mark_dirty(&current);
                self.directory_cache.invalidate(&current);
                if let Some(ref di) = self.directory_index {
                    let _ = di.invalidate(&current);
                }

                // Also invalidate the PARENT folder's caches (DirectoryCache +
                // DirectoryIndex).  When a file operation happens inside folder B,
                // Windows updates B's LastWriteTime.  But BOTH cache layers for the
                // parent folder A still hold B's old `modified` timestamp.  The
                // DirectoryIndex (SQLite) mtime validation compares A's own mtime
                // (which did NOT change) against the index timestamp, so it passes
                // and serves stale data.  Invalidating both layers forces a full
                // disk re-read when navigating back to A.
                if let Some(parent) = current.parent() {
                    log::info!("[FILE-OP] Invalidating cache for parent={:?}", parent);
                    let parent_buf = parent.to_path_buf();
                    self.directory_dirty_registry.mark_dirty(&parent_buf);
                    self.directory_cache.invalidate(&parent_buf);
                    if let Some(ref di) = self.directory_index {
                        let _ = di.invalidate(&parent_buf);
                    }
                }

                self.loaded_path.clear();
                self.reload_current_folder_preserving_icon_cache();

                // Also reload the inactive dual panel if it exists
                // (its folder may have been affected by the operation)
                if self.dual_panel_enabled {
                    if let Some(ref snapshot) = self.dual_panel_inactive_state {
                        let inactive_path = PathBuf::from(&snapshot.path);
                        self.directory_dirty_registry.mark_dirty(&inactive_path);
                        self.directory_cache.invalidate(&inactive_path);
                        if let Some(ref di) = self.directory_index {
                            let _ = di.invalidate(&inactive_path);
                        }
                    }
                    self.with_inactive_panel(|app| {
                        app.loaded_path.clear();
                        app.load_folder_for_inactive();
                    });
                }

                // Suppress watcher-triggered reloads for 2 seconds after the
                // forced reload. Archive extraction creates many files, causing the
                // watcher to fire events for seconds afterward. Without this
                // cooldown the status bar item count keeps flickering.
                self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
            }
        }
    }
}
