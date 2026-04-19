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
        self.app_state_db.remove_covers_for_path(&cleaned);
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
        let s = p.to_string_lossy().to_lowercase();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            stripped.to_string()
        } else {
            s
        }
    }

    pub(super) fn clean_path(p: &Path) -> PathBuf {
        let s = p.to_string_lossy();
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
        // A directory cache invalidation means the folder's contents may have
        // changed. Invalidate folder-size caches here too so callers do not
        // need to remember to clear size separately from cover/listing state.
        self.invalidate_folder_size_cache(path);
        self.invalidate_folder_cover_state(path);
    }

    pub(super) fn try_remove_deleted_path_from_ui(&mut self, path: &Path) -> bool {
        let path_to_remove = path.to_path_buf();
        let path_norm = Self::normalize_for_match(&path_to_remove);

        // Prevent unbounded growth of pending_deletions from rapid watcher events.
        // A single folder rarely contains more than 10 000 deletions between reloads.
        if self.file_operation_state.pending_deletions.len() < 10_000 {
            self.file_operation_state
                .pending_deletions
                .insert(path_to_remove.clone(), ());
        }
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

    pub(in crate::app) fn invalidate_folder_size_cache(&mut self, folder: &Path) {
        let folder_path = folder.to_path_buf();
        let was_loading = self.folder_size_state.loading.remove(&folder_path);
        self.folder_size_state.cache.pop(&folder_path);
        // Also clear the batch (list-view) cache so the next render re-fetches.
        self.folder_size_state.batch_cache.pop(&folder_path);
        self.folder_size_state.batch_loading.remove(&folder_path);

        // Bump per-path invalidation epoch so that any in-flight batch
        // worker result (carrying its request_epoch) is detected as stale
        // and discarded in process_folder_size_results.
        *self
            .folder_size_state
            .batch_invalidation_epoch
            .entry(folder_path.clone())
            .or_insert(0) += 1;

        // Schedule a deferred re-invalidation to handle the timing race with
        // the search service's USN journal polling (2 s interval).  If the
        // batch worker re-fetches before the service processes the deletion,
        // it will permanently cache a stale value.  The re-invalidation
        // clears it so the next render gets the updated size.
        self.folder_size_state.pending_revalidation.insert(
            folder_path,
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        );

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

    pub(super) fn should_ignore_watcher_path(
        &self,
        path: &Path,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
    ) -> bool {
        let cleaned = Self::clean_path(path);
        let cleaned_norm = Self::normalize_for_match(&cleaned);
        let is_internal_cache_event = match (internal_cache_root_norm, internal_cache_root_prefix) {
            (Some(root), Some(prefix)) => cleaned_norm == root || cleaned_norm.starts_with(prefix),
            _ => false,
        };

        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        is_internal_cache_event
            || name.starts_with("dumpstack.log")
            || name.starts_with("hiberfil.sys")
            || name.starts_with("pagefile.sys")
            || name.starts_with("swapfile.sys")
            || name == "desktop.ini"
            || name == "thumbs.db"
    }

    pub(super) fn apply_folder_content_change_invalidations(
        &mut self,
        folders_with_changed_contents: std::collections::HashSet<PathBuf>,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
    ) {
        if folders_with_changed_contents.is_empty() {
            return;
        }

        log::debug!(
            "[MTIME-SCHED] apply_folder_content_change_invalidations: {} folders changed, current_path={}",
            folders_with_changed_contents.len(),
            self.navigation_state.current_path
        );
        for fp in &folders_with_changed_contents {
            log::debug!("[MTIME-SCHED]   changed folder: {:?}", fp);
        }

        let current_path_norm =
            Self::normalize_for_match(Path::new(&self.navigation_state.current_path));
        for folder_path in &folders_with_changed_contents {
            self.invalidate_folder_size_cache(folder_path);
            self.cache_manager.invalidate_folder_preview(folder_path);
            self.scanned_folders.pop(folder_path);
            let folder_norm = Self::normalize_for_match(folder_path);
            if folder_norm != current_path_norm {
                self.directory_cache.invalidate(folder_path);
                self.clear_tab_cache_for_normalized_path(&folder_norm);
            }
            pending_disk_cache_invalidations.push(folder_path.clone());
            let _ = self.cover_worker_sender.send(folder_path.clone());

            if let Some(parent) = folder_path.parent() {
                let parent_buf = parent.to_path_buf();
                if !folders_with_changed_contents.contains(&parent_buf) {
                    let parent_norm = Self::normalize_for_match(parent);
                    self.directory_cache.invalidate(&parent_buf);
                    self.clear_tab_cache_for_normalized_path(&parent_norm);
                }
            }
        }

        let covers_to_evict: Vec<PathBuf> = self
            .all_items
            .iter()
            .filter(|item| {
                item.is_dir
                    && item.folder_cover.is_some()
                    && folders_with_changed_contents.contains(&item.path)
            })
            .filter_map(|item| item.folder_cover.clone())
            .collect();

        for cover in &covers_to_evict {
            self.cache_manager.texture_cache.pop(cover);
            self.cache_manager.loading_set.remove(cover);
        }

        let mut cleared_any = false;
        for item in &mut self.all_items {
            if item.is_dir
                && item.folder_cover.is_some()
                && folders_with_changed_contents.contains(&item.path)
            {
                item.folder_cover = None;
                cleared_any = true;
            }
        }
        if cleared_any {
            let items = Arc::make_mut(&mut self.items);
            for item in items.iter_mut() {
                if item.is_dir
                    && item.folder_cover.is_some()
                    && folders_with_changed_contents.contains(&item.path)
                {
                    item.folder_cover = None;
                }
            }
        }

        let mut scheduled_any = false;
        for folder_path in &folders_with_changed_contents {
            if crate::infrastructure::onedrive::is_onedrive_path(folder_path) {
                log::debug!(
                    "[MTIME-SCHED] Skipping OneDrive folder: {:?}",
                    folder_path
                );
                continue;
            }
            if crate::infrastructure::io_priority::is_network_or_virtual(folder_path) {
                log::debug!(
                    "[MTIME-SCHED] Skipping network/virtual folder: {:?}",
                    folder_path
                );
                continue;
            }

            let folder_norm = Self::normalize_for_match(folder_path);
            let is_visible = self.all_items.iter().any(|item| {
                item.is_dir
                    && (item.path == *folder_path
                        || Self::normalize_for_match(&item.path) == folder_norm)
            });
            if !is_visible {
                log::debug!(
                    "[MTIME-SCHED] Folder NOT visible in listing, skipping: {:?} (norm={:?})",
                    folder_path,
                    folder_norm
                );
                continue;
            }

            let recheck_at = Instant::now() + Duration::from_secs(2);
            if let Some(existing) = self
                .pending_folder_mtime_recheck
                .iter_mut()
                .find(|(p, _)| p == folder_path)
            {
                existing.1 = recheck_at;
                log::debug!(
                    "[MTIME-SCHED] Debounce push for folder: {:?}",
                    folder_path.file_name().unwrap_or_default()
                );
            } else {
                if self.pending_folder_mtime_recheck.len() >= 500 {
                    log::warn!(
                        "[MTIME-SCHED] Pending mtime recheck list full (500), dropping: {:?}",
                        folder_path.file_name().unwrap_or_default()
                    );
                    continue;
                }
                log::debug!(
                    "[MTIME-SCHED] Scheduled mtime recheck for folder: {:?} (due in 2s)",
                    folder_path.file_name().unwrap_or_default()
                );
                self.pending_folder_mtime_recheck
                    .push((folder_path.clone(), recheck_at));
            }
            scheduled_any = true;
        }

        if scheduled_any {
            self.ui_ctx
                .request_repaint_after(Duration::from_millis(2500));
        }
    }

    pub(super) fn process_pending_folder_mtime_rechecks(&mut self) {
        if self.pending_folder_mtime_recheck.is_empty() {
            return;
        }

        let now = Instant::now();

        const MTIME_SORT_COOLDOWN: Duration = Duration::from_secs(3);
        let cooldown_remaining = MTIME_SORT_COOLDOWN
            .checked_sub(now.duration_since(self.last_folder_mtime_sort))
            .unwrap_or(Duration::ZERO);
        if cooldown_remaining > Duration::ZERO {
            self.ui_ctx
                .request_repaint_after(cooldown_remaining + Duration::from_millis(50));
            return;
        }

        let due_entries: Vec<PathBuf> = self
            .pending_folder_mtime_recheck
            .iter()
            .filter(|(_, recheck_at)| now >= *recheck_at)
            .map(|(p, _)| p.clone())
            .collect();

        if due_entries.is_empty() {
            if let Some(earliest) = self
                .pending_folder_mtime_recheck
                .iter()
                .map(|(_, t)| *t)
                .min()
            {
                if let Some(wait) = earliest.checked_duration_since(now) {
                    self.ui_ctx
                        .request_repaint_after(wait + Duration::from_millis(50));
                }
            }
            return;
        }

        log::info!(
            "[MTIME-CHECK] Processing {} due folder mtime rechecks (pending={}, cooldown_ok)",
            due_entries.len(),
            self.pending_folder_mtime_recheck.len()
        );

        self.pending_folder_mtime_recheck
            .retain(|(_, recheck_at)| now < *recheck_at);

        let mut folder_mtime_updated = false;

        for folder_path in &due_entries {
            let new_modified = match std::fs::metadata(folder_path) {
                Ok(meta) if meta.is_dir() => meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                Ok(_) => {
                    log::info!(
                        "[MTIME-CHECK] Path is not a directory, skipping: {:?}",
                        folder_path
                    );
                    continue;
                }
                Err(e) => {
                    log::info!(
                        "[MTIME-CHECK] metadata() failed for {:?}: {}",
                        folder_path,
                        e
                    );
                    continue;
                }
            };

            if new_modified == 0 {
                log::info!(
                    "[MTIME-CHECK] mtime=0 for {:?}, skipping",
                    folder_path.file_name().unwrap_or_default()
                );
                continue;
            }

            let folder_norm = Self::normalize_for_match(folder_path);
            let mut this_updated = false;

            let old_modified = self
                .all_items
                .iter()
                .find(|item| {
                    item.is_dir
                        && (item.path == *folder_path
                            || Self::normalize_for_match(&item.path) == folder_norm)
                })
                .map(|item| item.modified)
                .unwrap_or(0);

            if old_modified == new_modified {
                log::info!(
                    "[MTIME-CHECK] mtime unchanged for {:?}: {} == {}",
                    folder_path.file_name().unwrap_or_default(),
                    old_modified,
                    new_modified
                );
                continue;
            }

            for item in self.all_items.iter_mut() {
                if item.is_dir
                    && (item.path == *folder_path
                        || Self::normalize_for_match(&item.path) == folder_norm)
                {
                    item.modified = new_modified;
                    this_updated = true;
                    break;
                }
            }

            if this_updated {
                let items = Arc::make_mut(&mut self.items);
                for item in items.iter_mut() {
                    if item.is_dir
                        && (item.path == *folder_path
                            || Self::normalize_for_match(&item.path) == folder_norm)
                    {
                        item.modified = new_modified;
                        break;
                    }
                }
                folder_mtime_updated = true;
                log::info!(
                    "[MTIME-CHECK] Updated folder mtime: {:?} {} → {}",
                    folder_path.file_name().unwrap_or_default(),
                    old_modified,
                    new_modified
                );
            }
        }

        if folder_mtime_updated {
            self.sort_items();
            self.ui_ctx.request_repaint();
            self.last_folder_mtime_sort = now;
            log::info!("[MTIME-CHECK] Re-sorted items after folder mtime update");

            let current_path_buf =
                PathBuf::from(&self.navigation_state.current_path);
            self.directory_cache.invalidate(&current_path_buf);
        }

        if !self.pending_folder_mtime_recheck.is_empty() {
            if let Some(earliest) = self
                .pending_folder_mtime_recheck
                .iter()
                .map(|(_, t)| *t)
                .min()
            {
                if let Some(wait) = earliest.checked_duration_since(now) {
                    self.ui_ctx
                        .request_repaint_after(wait + Duration::from_millis(50));
                }
            }
        }
    }
}
