use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    fn path_matches_normalized(candidate: &Path, target_norm: &str) -> bool {
        Self::normalize_for_match(candidate) == target_norm
    }

    pub(super) fn is_known_directory_path(&self, path: &Path) -> bool {
        if crate::infrastructure::windows::is_drive_root_path(path) {
            return true;
        }

        let path_norm = Self::normalize_for_match(path);

        self.all_items
            .iter()
            .any(|item| item.is_dir && Self::path_matches_normalized(&item.path, &path_norm))
            || self
                .items
                .iter()
                .any(|item| item.is_dir && Self::path_matches_normalized(&item.path, &path_norm))
            || self
                .selected_file
                .as_ref()
                .is_some_and(|item| {
                    item.is_dir && Self::path_matches_normalized(&item.path, &path_norm)
                })
    }

    pub(super) fn register_changed_folder_for_path(
        &self,
        changed_path: &Path,
        out: &mut std::collections::HashSet<PathBuf>,
    ) {
        let cleaned = Self::clean_path(changed_path);
        let is_known_dir = self.is_known_directory_path(&cleaned);

        if is_known_dir || cleaned.extension().is_none() {
            out.insert(cleaned.clone());
            if let Some(parent) = cleaned.parent() {
                out.insert(parent.to_path_buf());
            }
        } else if let Some(parent) = cleaned.parent() {
            out.insert(parent.to_path_buf());
        }
    }

    pub(super) fn evict_stale_path_caches(&mut self, path: &Path) {
        let cleaned = Self::clean_path(path);
        let remove_paths = vec![cleaned.clone()];

        self.thumbnail_queue.remove_paths(&remove_paths);
        self.cache_manager.texture_cache.pop(&cleaned);
        self.cache_manager.loading_set.remove(&cleaned);
        self.cache_manager.pop_rgba_data(&cleaned);
        self.cache_manager.failed_thumbnails.pop(&cleaned);
        self.metadata_cache.pop(&cleaned);
        self.live_file_size_cache.pop(&cleaned);
        self.disk_cache.remove_cache_for_path(&cleaned);
        crate::workers::thumbnail::clear_failure_cache(&cleaned);

        // Drain any stale thumbnail data from the GPU upload queue so it
        // cannot re-insert an outdated texture into texture_cache later
        // in the same (or next) frame.
        self.pending_thumbnails.retain(|t| t.path != cleaned);
        self.cache_manager.finish_pending_upload(&cleaned);

        // Track that one stale in-flight result may still be in the
        // worker-to-UI channel.  The upload pipeline will decrement
        // this counter and discard the matching result.
        *self.thumbnail_eviction_skips.entry(cleaned.clone()).or_insert(0) += 1;

        if self.last_metadata_path.as_ref() == Some(&cleaned) {
            self.last_metadata_path = None;
        }

        if matches!(self.selected_metadata.as_ref(), Some((p, _)) if *p == cleaned) {
            self.selected_metadata = None;
        }
    }

    pub(super) fn normalize_for_match(p: &Path) -> String {
        let s = p.to_string_lossy().to_string().to_lowercase();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            stripped.to_string()
        } else {
            s
        }
    }

    pub(super) fn clean_path(p: &Path) -> PathBuf {
        let s = p.to_string_lossy().to_string();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            p.to_path_buf()
        }
    }

    pub(super) fn invalidate_folder_cover_state(&mut self, folder: &Path) {
        let folder_path = folder.to_path_buf();

        self.cache_manager.invalidate_folder_preview(&folder_path);
        self.scanned_folders.pop(&folder_path);

        let mut stale_cover_paths = std::collections::HashSet::new();
        let mut updated_any = false;

        for item in &mut self.all_items {
            if item.is_dir && item.path == folder_path {
                if let Some(cover) = item.folder_cover.take() {
                    stale_cover_paths.insert(cover);
                }
                updated_any = true;
            }
        }

        let items = Arc::make_mut(&mut self.items);
        for item in items.iter_mut() {
            if item.is_dir && item.path == folder_path {
                if let Some(cover) = item.folder_cover.take() {
                    stale_cover_paths.insert(cover);
                }
                updated_any = true;
            }
        }

        for cover in stale_cover_paths {
            self.cache_manager.texture_cache.pop(&cover);
            self.cache_manager.loading_set.remove(&cover);
            self.cache_manager.pop_rgba_data(&cover);
            self.cache_manager.failed_thumbnails.pop(&cover);
        }

        self.enqueue_disk_cache_invalidations(vec![folder_path.clone()]);
        let _ = self.cover_worker_sender.send(folder_path);

        if updated_any {
            self.pending_items_rebuild = true;
        }
    }

    pub(super) fn invalidate_directory_caches(&mut self, path: &Path) {
        let path_buf = path.to_path_buf();
        self.directory_dirty_registry.mark_dirty(path);
        self.directory_cache.invalidate(&path_buf);
        if let Some(di) = &self.directory_index {
            let _ = di.invalidate(path);
        }
        self.invalidate_folder_cover_state(path);
    }

    pub(super) fn try_remove_deleted_path_from_ui(&mut self, path: &Path) -> bool {
        let path_to_remove = path.to_path_buf();
        let path_norm = Self::normalize_for_match(&path_to_remove);

        self.file_operation_state
            .pending_deletions
            .insert(path_to_remove.clone(), ());
        self.evict_stale_path_caches(&path_to_remove);
        self.enqueue_disk_cache_invalidations_forced(vec![path_to_remove.clone()]);

        let removed_from_all = self
            .all_items
            .iter()
            .position(|item| Self::path_matches_normalized(&item.path, &path_norm))
            .map(|idx| {
                self.all_items.remove(idx);
                true
            })
            .unwrap_or(false);

        if !removed_from_all {
            return false;
        }

        let items = Arc::make_mut(&mut self.items);
        if let Some(idx) = items
            .iter()
            .position(|item| Self::path_matches_normalized(&item.path, &path_norm))
        {
            items.remove(idx);
        }

        self.total_items = self.items.len();
        self.multi_selection
            .retain(|selected_path| !Self::path_matches_normalized(selected_path, &path_norm));

        if self
            .selected_file
            .as_ref()
            .is_some_and(|selected| Self::path_matches_normalized(&selected.path, &path_norm))
        {
            self.selected_file = None;
            self.selected_thumbnail = None;
            self.selected_metadata = None;
        }

        if let Some(selected) = self.selected_item {
            if selected >= self.items.len() && !self.items.is_empty() {
                self.selected_item = Some(self.items.len() - 1);
            } else if self.items.is_empty() {
                self.selected_item = None;
                self.selected_file = None;
            }
        }

        true
    }

    pub(super) fn try_add_created_path_to_ui(&mut self, path: &Path) -> bool {
        let cleaned = Self::clean_path(path);
        let cleaned_norm = Self::normalize_for_match(&cleaned);

        if crate::infrastructure::onedrive::is_onedrive_path(&cleaned)
            || crate::infrastructure::io_priority::is_network_or_virtual(&cleaned)
        {
            return false;
        }

        self.file_operation_state.pending_deletions.remove(&cleaned);
        self.evict_stale_path_caches(&cleaned);
        self.enqueue_disk_cache_invalidations_forced(vec![cleaned.clone()]);

        let is_dir = match std::fs::metadata(&cleaned) {
            Ok(metadata) => metadata.is_dir(),
            Err(_) => return false,
        };

        let new_entry = FileEntry::from_path(cleaned.clone(), is_dir);

        if let Some(idx) = self
            .all_items
            .iter()
            .position(|item| Self::path_matches_normalized(&item.path, &cleaned_norm))
        {
            self.all_items[idx] = new_entry;
        } else {
            self.all_items.push(new_entry);
        }

        self.filter_items();
        self.sort_items();
        if is_dir {
            self.request_folder_scan(cleaned.clone());
        }
        self.ui_ctx.request_repaint();
        true
    }

    pub(super) fn try_apply_rename_to_ui(&mut self, old_path: &Path, new_path: &Path) -> bool {
        let cleaned_old = Self::clean_path(old_path);
        let cleaned_new = Self::clean_path(new_path);
        let old_norm = Self::normalize_for_match(&cleaned_old);
        let new_norm = Self::normalize_for_match(&cleaned_new);
        let old_was_selected = self
            .selected_file
            .as_ref()
            .is_some_and(|selected| Self::normalize_for_match(&selected.path) == old_norm);

        // Mark old path as pending deletion (remove from pending if new
        // path was previously pending — handles the OneDrive early-return
        // case in try_add_created_path_to_ui).
        self.file_operation_state
            .pending_deletions
            .insert(cleaned_old.clone(), ());
        self.file_operation_state.pending_deletions.remove(&cleaned_new);

        // NOTE: evict_stale_path_caches / enqueue_disk_cache_invalidations_forced
        // are called inside the sub-methods below.  Calling them here too would
        // double-increment thumbnail_eviction_skips and queue redundant invalidations.
        let removed_old = self.try_remove_deleted_path_from_ui(&cleaned_old);
        let added_new = self.try_add_created_path_to_ui(&cleaned_new);

        if old_was_selected {
            if let Some(idx) = self
                .items
                .iter()
                .position(|item| Self::path_matches_normalized(&item.path, &new_norm))
            {
                self.selected_item = Some(idx);
                self.selected_file = Some(self.items[idx].clone());
                self.selected_thumbnail = None;
                self.selected_metadata = None;
            }
        }

        removed_old || added_new
    }

    pub(super) fn request_watcher_auto_reload(&mut self) {
        if !self.pending_auto_reload {
            self.last_auto_reload = Instant::now();
            // Request repaint at exactly the debounce deadline so the reload
            // fires as soon as the debounce elapses, regardless of idle state.
            self.ui_ctx.request_repaint_after(Duration::from_millis(
                crate::ui::theme::AUTO_RELOAD_MS.saturating_add(25),
            ));
        }
        self.pending_auto_reload = true;
    }

    pub(super) fn invalidate_folder_size_cache(&mut self, folder: &Path) {
        let folder_path = folder.to_path_buf();
        let was_loading = self.folder_size_state.loading.remove(&folder_path);
        self.folder_size_state.cache.pop(&folder_path);

        if was_loading {
            self.folder_size_state.cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn clear_tab_cache_for_normalized_path(&mut self, path_norm: &str) {
        for tab in self.tab_manager.tabs.iter_mut() {
            let tab_path = Self::normalize_for_match(Path::new(&tab.path));
            if tab_path == path_norm {
                tab.items = Arc::new(Vec::new());
                tab.all_items.clear();
            }
        }
    }
}
