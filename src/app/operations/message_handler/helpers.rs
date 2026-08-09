use crate::app::state::ImageViewerApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const FOLDER_COVER_REFRESH_DEBOUNCE: Duration = Duration::from_secs(2);
const MAX_PENDING_FOLDER_COVER_REFRESHES: usize = 500;
const MAX_PENDING_FOLDER_COVER_REMOVALS: usize = 256;
const MAX_FOLDER_COVER_REMOVE_ATTEMPTS_PER_FRAME: usize = 8;
const FOLDER_COVER_REMOVE_FRAME_BUDGET: Duration = Duration::from_millis(2);
const FOLDER_COVER_REMOVE_SHUTDOWN_BUDGET: Duration = Duration::from_millis(40);
const FOLDER_COVER_REMOVE_SHUTDOWN_BUSY_RETRY: Duration = Duration::from_millis(1);
const FOLDER_COVER_REMOVE_BUSY_RETRY: Duration = Duration::from_millis(100);
const FOLDER_COVER_REMOVE_FAILED_RETRY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebounceQueueAction {
    Inserted,
    Updated,
    Dropped,
}

fn upsert_debounced_path(
    entries: &mut Vec<(PathBuf, Instant)>,
    path: &Path,
    due_at: Instant,
    max_entries: usize,
) -> DebounceQueueAction {
    if let Some(existing) = entries
        .iter_mut()
        .find(|(existing_path, _)| existing_path.as_path() == path)
    {
        existing.1 = due_at;
        DebounceQueueAction::Updated
    } else if entries.len() >= max_entries {
        DebounceQueueAction::Dropped
    } else {
        entries.push((path.to_path_buf(), due_at));
        DebounceQueueAction::Inserted
    }
}

fn take_due_debounced_paths(entries: &mut Vec<(PathBuf, Instant)>, now: Instant) -> Vec<PathBuf> {
    let due_entries: Vec<PathBuf> = entries
        .iter()
        .filter(|(_, refresh_at)| now >= *refresh_at)
        .map(|(path, _)| path.clone())
        .collect();

    if !due_entries.is_empty() {
        entries.retain(|(_, refresh_at)| now < *refresh_at);
    }

    due_entries
}

fn upsert_pending_folder_cover_removal(
    entries: &mut std::collections::BTreeMap<PathBuf, (Instant, bool)>,
    path: &Path,
    retry_at: Instant,
    rescan_after_remove: bool,
    max_entries: usize,
) -> DebounceQueueAction {
    if let Some((existing_retry_at, existing_rescan)) = entries.get_mut(path) {
        *existing_retry_at = (*existing_retry_at).min(retry_at);
        *existing_rescan |= rescan_after_remove;
        DebounceQueueAction::Updated
    } else if entries.len() >= max_entries {
        // Keep the existing set. This makes overload behavior independent of
        // BTreeMap iteration order and prevents old retries from being starved.
        DebounceQueueAction::Dropped
    } else {
        entries.insert(path.to_path_buf(), (retry_at, rescan_after_remove));
        DebounceQueueAction::Inserted
    }
}

fn run_folder_cover_removals_with_budget<Clock, Attempt>(
    entries: &mut std::collections::BTreeMap<PathBuf, (Instant, bool)>,
    due_by: Option<Instant>,
    max_attempts: usize,
    deadline: Instant,
    mut clock: Clock,
    mut attempt: Attempt,
) -> usize
where
    Clock: FnMut() -> Instant,
    Attempt: FnMut(&Path, bool) -> Option<Instant>,
{
    let mut candidates: Vec<(Instant, PathBuf)> = entries
        .iter()
        .filter(|(_, (retry_at, _))| due_by.is_none_or(|now| *retry_at <= now))
        .map(|(path, (retry_at, _))| (*retry_at, path.clone()))
        .collect();
    candidates.sort_unstable_by(|(left_due, left_path), (right_due, right_path)| {
        left_due
            .cmp(right_due)
            .then_with(|| left_path.cmp(right_path))
    });
    candidates.truncate(max_attempts);

    let mut attempts = 0;
    for (_, path) in candidates {
        if clock() >= deadline {
            break;
        }

        let Some((_, rescan_after_remove)) = entries.remove(&path) else {
            continue;
        };
        if let Some(retry_at) = attempt(&path, rescan_after_remove) {
            let action = upsert_pending_folder_cover_removal(
                entries,
                &path,
                retry_at,
                rescan_after_remove,
                MAX_PENDING_FOLDER_COVER_REMOVALS,
            );
            debug_assert_ne!(action, DebounceQueueAction::Dropped);
        }
        attempts += 1;
    }

    attempts
}

impl ImageViewerApp {
    fn finish_folder_cover_removal(&self, folder_path: &Path, rescan_after_remove: bool) {
        if rescan_after_remove {
            let _ = self.cover_worker_sender.send(folder_path.to_path_buf());
        }
    }

    fn defer_folder_cover_removal(
        &mut self,
        folder_path: &Path,
        retry_after: Duration,
        rescan_after_remove: bool,
    ) {
        let retry_at = Instant::now() + retry_after;
        let action = upsert_pending_folder_cover_removal(
            &mut self.pending_folder_cover_removals,
            folder_path,
            retry_at,
            rescan_after_remove,
            MAX_PENDING_FOLDER_COVER_REMOVALS,
        );
        if action == DebounceQueueAction::Dropped {
            log::debug!(
                "[FOLDER-COVERS] Removal retry queue full; leaving {:?} for future revalidation",
                folder_path
            );
        }
        self.schedule_next_folder_cover_removal_repaint();
    }

    fn schedule_next_folder_cover_removal_repaint(&self) {
        if let Some(next_retry) = self
            .pending_folder_cover_removals
            .values()
            .map(|(retry_at, _)| *retry_at)
            .min()
        {
            self.ui_ctx
                .request_repaint_after(next_retry.saturating_duration_since(Instant::now()));
        }
    }

    pub(crate) fn remove_folder_cover_without_blocking(
        &mut self,
        folder_path: &Path,
        rescan_after_remove: bool,
    ) {
        use crate::infrastructure::app_state_db::FolderCoverRemoveOutcome;

        if let Some((_, pending_rescan)) = self.pending_folder_cover_removals.get_mut(folder_path) {
            *pending_rescan |= rescan_after_remove;
            self.schedule_next_folder_cover_removal_repaint();
            return;
        }

        match self.app_state_db.try_remove_folder_cover(folder_path) {
            FolderCoverRemoveOutcome::Removed => {
                self.finish_folder_cover_removal(folder_path, rescan_after_remove);
            }
            FolderCoverRemoveOutcome::Busy => {
                self.defer_folder_cover_removal(
                    folder_path,
                    FOLDER_COVER_REMOVE_BUSY_RETRY,
                    rescan_after_remove,
                );
            }
            FolderCoverRemoveOutcome::Failed(error) => {
                log::warn!(
                    "[FOLDER-COVERS] Failed to remove cover for {:?}: {error}; retrying",
                    folder_path
                );
                self.defer_folder_cover_removal(
                    folder_path,
                    FOLDER_COVER_REMOVE_FAILED_RETRY,
                    rescan_after_remove,
                );
            }
        }
    }

    pub(super) fn process_pending_folder_cover_removals(&mut self) {
        if self.pending_folder_cover_removals.is_empty() {
            return;
        }

        use crate::infrastructure::app_state_db::FolderCoverRemoveOutcome;

        let now = Instant::now();
        let deadline = now + FOLDER_COVER_REMOVE_FRAME_BUDGET;
        let app_state_db = Arc::clone(&self.app_state_db);
        let cover_worker_sender = self.cover_worker_sender.clone();
        run_folder_cover_removals_with_budget(
            &mut self.pending_folder_cover_removals,
            Some(now),
            MAX_FOLDER_COVER_REMOVE_ATTEMPTS_PER_FRAME,
            deadline,
            Instant::now,
            |folder_path, rescan_after_remove| match app_state_db
                .try_remove_folder_cover(folder_path)
            {
                FolderCoverRemoveOutcome::Removed => {
                    if rescan_after_remove {
                        let _ = cover_worker_sender.send(folder_path.to_path_buf());
                    }
                    None
                }
                FolderCoverRemoveOutcome::Busy => {
                    Some(Instant::now() + FOLDER_COVER_REMOVE_BUSY_RETRY)
                }
                FolderCoverRemoveOutcome::Failed(error) => {
                    log::warn!(
                        "[FOLDER-COVERS] Retry failed for {:?}: {error}",
                        folder_path
                    );
                    Some(Instant::now() + FOLDER_COVER_REMOVE_FAILED_RETRY)
                }
            },
        );

        self.schedule_next_folder_cover_removal_repaint();
    }

    pub(crate) fn flush_pending_folder_cover_removals_for_shutdown(&mut self) {
        if self.pending_folder_cover_removals.is_empty() {
            return;
        }

        use crate::infrastructure::app_state_db::FolderCoverRemoveOutcome;

        let started = Instant::now();
        let deadline = started + FOLDER_COVER_REMOVE_SHUTDOWN_BUDGET;
        let app_state_db = Arc::clone(&self.app_state_db);
        let cover_worker_sender = self.cover_worker_sender.clone();
        for (retry_at, _) in self.pending_folder_cover_removals.values_mut() {
            *retry_at = started;
        }

        while !self.pending_folder_cover_removals.is_empty() && Instant::now() < deadline {
            let now = Instant::now();
            let attempts = run_folder_cover_removals_with_budget(
                &mut self.pending_folder_cover_removals,
                Some(now),
                MAX_PENDING_FOLDER_COVER_REMOVALS,
                deadline,
                Instant::now,
                |folder_path, rescan_after_remove| match app_state_db
                    .try_remove_folder_cover(folder_path)
                {
                    FolderCoverRemoveOutcome::Removed => {
                        if rescan_after_remove {
                            let _ = cover_worker_sender.send(folder_path.to_path_buf());
                        }
                        None
                    }
                    FolderCoverRemoveOutcome::Busy => {
                        Some(Instant::now() + FOLDER_COVER_REMOVE_SHUTDOWN_BUSY_RETRY)
                    }
                    FolderCoverRemoveOutcome::Failed(error) => {
                        log::warn!(
                            "[FOLDER-COVERS] Shutdown removal failed for {:?}: {error}",
                            folder_path
                        );
                        Some(Instant::now() + FOLDER_COVER_REMOVE_FAILED_RETRY)
                    }
                },
            );

            let Some(next_retry) = self
                .pending_folder_cover_removals
                .values()
                .map(|(retry_at, _)| *retry_at)
                .min()
            else {
                break;
            };
            if next_retry >= deadline {
                break;
            }
            if attempts == 0 || next_retry > Instant::now() {
                std::thread::sleep(
                    next_retry
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(1)),
                );
            }
        }

        if !self.pending_folder_cover_removals.is_empty() {
            log::warn!(
                "[FOLDER-COVERS] Shutdown flush left {} removal(s) for future revalidation after {:?}",
                self.pending_folder_cover_removals.len(),
                started.elapsed()
            );
        }
    }

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
            || self.selected_file.as_ref().is_some_and(|item| {
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
        self.cache_manager
            .forget_attempted_thumbnail_bucket(&cleaned);
        self.cache_manager.loading_set.remove(&cleaned);
        self.cache_manager.pop_rgba_data(&cleaned);
        self.cache_manager.failed_thumbnails.pop(&cleaned);
        self.metadata_cache.pop(&cleaned);
        crate::app::live_file_size::invalidate_live_file_size(
            &cleaned,
            &mut self.live_file_size_cache,
            &mut self.live_file_size_loading,
        );
        self.disk_cache.remove_cache_for_path(&cleaned);
        self.app_state_db.remove_covers_for_path(&cleaned);
        crate::workers::thumbnail::clear_failure_cache(&cleaned);

        // Drain any stale thumbnail data from the GPU upload queue so it
        // cannot re-insert an outdated texture into texture_cache later
        // in the same (or next) frame.
        self.pending_thumbnails.retain(|t| t.path != cleaned);
        self.cache_manager.finish_pending_upload(&cleaned);

        self.bump_thumbnail_request_epoch(&cleaned);

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

    pub(super) fn refresh_cached_sync_status_for_path(&mut self, path: &Path) -> bool {
        let cleaned = Self::clean_path(path);
        let Some(status) = crate::infrastructure::onedrive::sync_status_for_path(&cleaned) else {
            return false;
        };

        let path_norm = Self::normalize_for_match(&cleaned);
        let mut updated_any = false;

        for item in self.all_items_mut().iter_mut() {
            if Self::path_matches_normalized(&item.path, &path_norm) && item.sync_status != status {
                item.sync_status = status;
                updated_any = true;
            }
        }

        let items = Arc::make_mut(&mut self.items);
        for item in items.iter_mut() {
            if Self::path_matches_normalized(&item.path, &path_norm) && item.sync_status != status {
                item.sync_status = status;
                updated_any = true;
            }
        }

        if let Some(selected) = self.selected_file.as_mut() {
            if Self::path_matches_normalized(&selected.path, &path_norm)
                && selected.sync_status != status
            {
                selected.sync_status = status;
                updated_any = true;
            }
        }

        if updated_any {
            self.ui_ctx.request_repaint();
        }

        updated_any
    }

    pub(super) fn invalidate_folder_cover_state(&mut self, folder: &Path) {
        let folder_path = folder.to_path_buf();

        self.cache_manager.invalidate_folder_preview(&folder_path);
        self.scanned_folders.pop(&folder_path);

        let mut stale_cover_paths = std::collections::HashSet::new();
        let mut updated_any = false;

        for item in self.all_items_mut().iter_mut() {
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
        self.remove_folder_cover_without_blocking(&folder_path, true);

        if updated_any {
            self.pending_items_rebuild = true;
        }
    }

    pub(super) fn invalidate_directory_caches(&mut self, path: &Path) {
        self.invalidate_directory_listing_caches(path);
        self.invalidate_folder_cover_state(path);
    }

    pub(super) fn invalidate_directory_listing_caches(&mut self, path: &Path) {
        let path_buf = path.to_path_buf();
        self.directory_dirty_registry.mark_dirty(path);
        self.directory_cache.invalidate(&path_buf);
        if let Some(di) = &self.directory_index {
            let _ = di.invalidate(path);
        }
        self.miller_columns.invalidate(path);
        if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
            snapshot.miller_columns.invalidate(path);
        }
        // A directory cache invalidation means the folder's contents may have
        // changed. Invalidate folder-size caches here too so callers do not
        // need to remember to clear size separately from cover/listing state.
        self.invalidate_folder_size_cache(path);
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
        // Drop the persistent FileEntry cache row so the next tag view load
        // does not surface a deleted/missing file. Idempotent: invalidate is
        // a DELETE so a missing row is a no-op.
        self.app_state_db
            .invalidate_cached_file_entry(&path_to_remove);

        let removed_from_all = self
            .all_items
            .iter()
            .position(|item| Self::path_matches_normalized(&item.path, &path_norm))
            .map(|idx| {
                self.all_items_mut().remove(idx);
                true
            })
            .unwrap_or(false);

        if !removed_from_all {
            return false;
        }
        self.invalidate_and_schedule_items_rebuild();

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

        self.rebuild_group_projection();
        self.reconcile_visible_selection_index();

        true
    }

    /// Batch variant of try_remove_deleted_path_from_ui: removes multiple paths
    /// from all_items/items in a single O(n) pass instead of O(n*m) per-path scans.
    ///
    /// Used by the watcher during REMOVE event floods to avoid repeated linear
    /// searches when deleting many files at once.
    pub(super) fn batch_remove_deleted_paths_from_ui(&mut self, paths: &[PathBuf]) -> bool {
        if paths.is_empty() {
            return false;
        }

        let norms: std::collections::HashSet<String> =
            paths.iter().map(|p| Self::normalize_for_match(p)).collect();

        for path in paths {
            if self.file_operation_state.pending_deletions.len() < 10_000 {
                self.file_operation_state
                    .pending_deletions
                    .insert(path.clone(), ());
            }
            self.evict_stale_path_caches(path);
        }
        self.enqueue_disk_cache_invalidations_forced(paths.to_vec());

        let had_any = self
            .all_items
            .iter()
            .any(|item| norms.contains(&Self::normalize_for_match(&item.path)));

        if !had_any {
            return false;
        }
        self.invalidate_and_schedule_items_rebuild();

        self.all_items_mut()
            .retain(|item| !norms.contains(&Self::normalize_for_match(&item.path)));

        let items = Arc::make_mut(&mut self.items);
        items.retain(|item| !norms.contains(&Self::normalize_for_match(&item.path)));

        self.total_items = self.items.len();
        self.multi_selection
            .retain(|selected_path| !norms.contains(&Self::normalize_for_match(selected_path)));

        if self
            .selected_file
            .as_ref()
            .is_some_and(|selected| norms.contains(&Self::normalize_for_match(&selected.path)))
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

        self.rebuild_group_projection();
        self.reconcile_visible_selection_index();

        true
    }

    pub(super) fn try_add_created_path_to_ui(&mut self, path: &Path) -> bool {
        let cleaned = Self::clean_path(path);

        if crate::infrastructure::onedrive::is_cloud_sync_path(&cleaned)
            || crate::infrastructure::io_priority::is_network_or_virtual(&cleaned)
        {
            return false;
        }

        if !crate::infrastructure::onedrive::fast_path_exists(&cleaned) {
            return false;
        }

        let path_norm = Self::normalize_for_match(&cleaned);
        if self
            .all_items
            .iter()
            .any(|item| Self::path_matches_normalized(&item.path, &path_norm))
        {
            return true;
        }

        self.file_operation_state.pending_deletions.remove(&cleaned);
        self.thumbnail_queue
            .remove_paths(std::slice::from_ref(&cleaned));
        self.cache_manager.texture_cache.pop(&cleaned);
        self.cache_manager
            .forget_attempted_thumbnail_bucket(&cleaned);
        self.cache_manager.loading_set.remove(&cleaned);
        self.cache_manager.pop_rgba_data(&cleaned);
        self.cache_manager.failed_thumbnails.pop(&cleaned);
        self.metadata_cache.pop(&cleaned);
        crate::app::live_file_size::invalidate_live_file_size(
            &cleaned,
            &mut self.live_file_size_cache,
            &mut self.live_file_size_loading,
        );
        self.pending_thumbnails.retain(|t| t.path != cleaned);
        self.cache_manager.finish_pending_upload(&cleaned);
        crate::workers::thumbnail::clear_failure_cache(&cleaned);

        let is_dir = crate::infrastructure::onedrive::fast_is_dir(&cleaned);
        let entry = crate::domain::file_entry::FileEntry::from_path(cleaned.clone(), is_dir);

        self.invalidate_and_schedule_items_rebuild();
        self.all_items_mut().push(entry.clone());

        let should_show = self.search_query.is_empty()
            || entry
                .name
                .to_lowercase()
                .contains(&self.search_query.to_lowercase());

        if should_show {
            Arc::make_mut(&mut self.items).push(entry);
            self.total_items = self.items.len();
            self.rebuild_group_projection();
        }

        self.pending_items_rebuild = true;
        self.pending_items_count = self.pending_items_count.saturating_add(1);
        self.ui_ctx.request_repaint();

        log::debug!(
            "[FS-WATCH] Added created path to UI incrementally: {:?}",
            cleaned
        );
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
        self.file_operation_state
            .pending_deletions
            .remove(&cleaned_new);

        // Move the persistent FileEntry cache row so tag view loads after a
        // rename serve from the renamed path without re-stat. Done BEFORE
        // try_remove_deleted_path_from_ui so the row survives (its entry under
        // cleaned_old is moved to cleaned_new before the delete invalidates
        // cleaned_old).
        self.app_state_db
            .rename_cached_file_entry(&cleaned_old, &cleaned_new);

        // NOTE: evict_stale_path_caches / enqueue_disk_cache_invalidations_forced
        // are called inside the sub-methods below.  Calling them here too would
        // double-bump thumbnail_request_epochs and queue redundant invalidations.
        let removed_old = self.try_remove_deleted_path_from_ui(&cleaned_old);
        let added_new = self.try_add_created_path_to_ui(&cleaned_new);
        if added_new {
            self.rebuild_group_projection();
        }

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
        self.invalidate_folder_size_cache_with_options(folder, true);
    }

    pub(in crate::app) fn invalidate_folder_size_cache_without_revalidation(
        &mut self,
        folder: &Path,
    ) {
        self.invalidate_folder_size_cache_with_options(folder, false);
    }

    fn invalidate_folder_size_cache_with_options(
        &mut self,
        folder: &Path,
        schedule_revalidation: bool,
    ) {
        let folder_path = folder.to_path_buf();
        let now = std::time::Instant::now();
        let stale_total_size = self
            .folder_size_state
            .cache
            .peek(&folder_path)
            .map(|summary| summary.total_size)
            .or_else(|| {
                self.folder_size_state
                    .batch_cache
                    .peek(&folder_path)
                    .copied()
            })
            .or_else(|| {
                self.folder_size_state
                    .panel_stale_cache
                    .peek(&folder_path)
                    .map(|summary| summary.total_size)
            });
        if schedule_revalidation && stale_total_size.is_some() {
            let wake_panel_revalidation =
                if let Some(summary) = self.folder_size_state.cache.peek(&folder_path).copied() {
                    let has_counts = summary.has_counts();
                    self.folder_size_state
                        .preserve_panel_summary_for_deferred_revalidation(
                            folder_path.clone(),
                            summary,
                            now,
                        );
                    has_counts
                } else {
                    let has_stale = self
                        .folder_size_state
                        .panel_stale_cache
                        .contains(&folder_path);
                    self.folder_size_state
                        .reschedule_panel_revalidation_if_stale(&folder_path, now);
                    has_stale
                };
            if wake_panel_revalidation {
                self.ui_ctx.request_repaint_after(
                    crate::app::folder_size_state::PANEL_STALE_REVALIDATION_DELAY
                        + std::time::Duration::from_millis(25),
                );
            }
        }
        let was_loading = self.folder_size_state.cancel_active_request(&folder_path);
        self.folder_size_state.cache.pop(&folder_path);
        self.folder_size_state.clear_failure(&folder_path);
        // Also clear the batch (list-view) cache so the next render re-fetches.
        self.folder_size_state.batch_cache.pop(&folder_path);

        // Bump per-path invalidation epoch so that any in-flight batch
        // worker result (carrying its request_epoch) is detected as stale
        // and discarded in process_folder_size_results.
        *self
            .folder_size_state
            .batch_invalidation_epoch
            .entry(folder_path.clone())
            .or_insert(0) += 1;

        if schedule_revalidation && stale_total_size.is_some() {
            // The service validates the live directory identity. This single
            // delayed pass only covers changes still in transit through USN.
            let delay =
                self.folder_size_state
                    .schedule_revalidation(folder_path, stale_total_size, now);
            self.ui_ctx
                .request_repaint_after(delay + std::time::Duration::from_millis(25));
        } else if !schedule_revalidation {
            self.folder_size_state.cancel_revalidation(&folder_path);
        }

        if was_loading {
            self.folder_size_state.cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn clear_tab_cache_for_normalized_path(&mut self, path_norm: &str) {
        for tab in self.tab_manager.tabs.iter_mut() {
            let tab_path = Self::normalize_for_match(Path::new(&tab.path));
            if tab_path == path_norm {
                tab.items = Arc::new(Vec::new());
                tab.all_items = Arc::new(Vec::new());
                tab.items_revision = tab.items_revision.wrapping_add(1);
                tab.group_projection = Arc::new(Default::default());
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
            let folder_norm = Self::normalize_for_match(folder_path);
            if folder_norm != current_path_norm {
                self.directory_cache.invalidate(folder_path);
                self.clear_tab_cache_for_normalized_path(&folder_norm);
            }
            pending_disk_cache_invalidations.push(folder_path.clone());
            self.schedule_folder_cover_refresh(folder_path);

            if let Some(parent) = folder_path.parent() {
                let parent_buf = parent.to_path_buf();
                if !folders_with_changed_contents.contains(&parent_buf) {
                    let parent_norm = Self::normalize_for_match(parent);
                    self.directory_cache.invalidate(&parent_buf);
                    self.clear_tab_cache_for_normalized_path(&parent_norm);
                }
            }
        }

        let mut scheduled_any = false;
        for folder_path in &folders_with_changed_contents {
            if crate::infrastructure::onedrive::is_cloud_sync_path(folder_path) {
                log::debug!(
                    "[MTIME-SCHED] Skipping cloud sync folder: {:?}",
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

    pub(crate) fn schedule_folder_cover_refresh(&mut self, folder_path: &Path) {
        let refresh_at = Instant::now() + FOLDER_COVER_REFRESH_DEBOUNCE;

        match upsert_debounced_path(
            &mut self.pending_folder_cover_refresh,
            folder_path,
            refresh_at,
            MAX_PENDING_FOLDER_COVER_REFRESHES,
        ) {
            DebounceQueueAction::Updated => {
                log::debug!(
                    "[FOLDER-COVER-SCHED] Debounce push for folder: {:?}",
                    folder_path.file_name().unwrap_or_default()
                );
            }
            DebounceQueueAction::Inserted => {
                log::debug!(
                    "[FOLDER-COVER-SCHED] Scheduled cover refresh for folder: {:?} (due in {}ms)",
                    folder_path.file_name().unwrap_or_default(),
                    FOLDER_COVER_REFRESH_DEBOUNCE.as_millis()
                );
            }
            DebounceQueueAction::Dropped => {
                log::warn!(
                    "[FOLDER-COVER-SCHED] Pending cover refresh list full ({}), dropping: {:?}",
                    MAX_PENDING_FOLDER_COVER_REFRESHES,
                    folder_path.file_name().unwrap_or_default()
                );
                return;
            }
        }

        self.ui_ctx
            .request_repaint_after(FOLDER_COVER_REFRESH_DEBOUNCE + Duration::from_millis(500));
    }

    pub(super) fn process_pending_folder_cover_refreshes(&mut self) {
        if self.pending_folder_cover_refresh.is_empty() {
            return;
        }

        let now = Instant::now();
        let due_entries = take_due_debounced_paths(&mut self.pending_folder_cover_refresh, now);

        if due_entries.is_empty() {
            if let Some(earliest) = self
                .pending_folder_cover_refresh
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

        for folder_path in &due_entries {
            if self.cache_manager.has_folder_preview(folder_path) {
                // Keep the current composed preview visible during the write burst.
                // When the folder goes quiet, re-compose in the background and
                // swap the texture only when the new pixels are ready.
                self.request_folder_preview_refresh_preserving_current(folder_path.clone());
            }
            let _ = self.cover_worker_sender.send(folder_path.clone());
        }

        self.ui_ctx.request_repaint();

        if let Some(earliest) = self
            .pending_folder_cover_refresh
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

        if self.file_operation_state.file_ops_in_progress > 0 {
            self.ui_ctx
                .request_repaint_after(Duration::from_millis(500));
            return;
        }

        log::info!(
            "[MTIME-CHECK] Processing {} due folder mtime rechecks via async folder reload (pending={}, cooldown_ok)",
            due_entries.len(),
            self.pending_folder_mtime_recheck.len()
        );

        self.pending_folder_mtime_recheck
            .retain(|(_, recheck_at)| now < *recheck_at);

        if !due_entries.is_empty()
            && !self.is_loading_folder
            && !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
        {
            let current_path_buf = PathBuf::from(&self.navigation_state.current_path);
            self.directory_dirty_registry.mark_dirty(&current_path_buf);
            self.directory_cache.invalidate(&current_path_buf);
            if let Some(di) = &self.directory_index {
                let _ = di.invalidate(&current_path_buf);
            }
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
            self.last_folder_mtime_sort = now;
            self.ui_ctx.request_repaint();
            log::info!(
                "[MTIME-CHECK] Reloaded current folder asynchronously after deferred mtime event"
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_cover_refresh_upsert_updates_existing_deadline() {
        let path = PathBuf::from(r"C:\Temp\Dest");
        let first_due = Instant::now() + Duration::from_millis(250);
        let second_due = first_due + Duration::from_secs(2);
        let mut entries = Vec::new();

        assert_eq!(
            upsert_debounced_path(&mut entries, &path, first_due, 8),
            DebounceQueueAction::Inserted
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, path);
        assert_eq!(entries[0].1, first_due);

        let same_path = entries[0].0.clone();

        assert_eq!(
            upsert_debounced_path(&mut entries, &same_path, second_due, 8),
            DebounceQueueAction::Updated
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, second_due);
    }

    #[test]
    fn folder_cover_refresh_upsert_drops_when_queue_is_full() {
        let now = Instant::now();
        let mut entries = vec![(PathBuf::from(r"C:\Temp\One"), now)];

        assert_eq!(
            upsert_debounced_path(
                &mut entries,
                Path::new(r"C:\Temp\Two"),
                now + Duration::from_secs(1),
                1,
            ),
            DebounceQueueAction::Dropped
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, PathBuf::from(r"C:\Temp\One"));
    }

    #[test]
    fn folder_cover_refresh_take_due_paths_retains_future_entries() {
        let now = Instant::now();
        let due_path = PathBuf::from(r"C:\Temp\Due");
        let future_path = PathBuf::from(r"C:\Temp\Future");
        let future_due = now + Duration::from_secs(5);
        let mut entries = vec![
            (due_path.clone(), now - Duration::from_millis(1)),
            (future_path.clone(), future_due),
        ];

        let due_entries = take_due_debounced_paths(&mut entries, now);

        assert_eq!(due_entries, vec![due_path]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, future_path);
        assert_eq!(entries[0].1, future_due);
    }

    #[test]
    fn folder_cover_removal_retry_preserves_rescan_and_earliest_deadline() {
        let path = PathBuf::from(r"C:\Temp\Folder");
        let first_due = Instant::now() + Duration::from_secs(2);
        let earlier_due = first_due - Duration::from_secs(1);
        let mut entries = std::collections::BTreeMap::new();

        assert_eq!(
            upsert_pending_folder_cover_removal(&mut entries, &path, first_due, false, 8),
            DebounceQueueAction::Inserted
        );
        assert_eq!(
            upsert_pending_folder_cover_removal(&mut entries, &path, earlier_due, true, 8),
            DebounceQueueAction::Updated
        );

        assert_eq!(entries.get(&path), Some(&(earlier_due, true)));
    }

    #[test]
    fn folder_cover_removal_queue_drops_new_paths_at_cap_but_still_coalesces() {
        let now = Instant::now();
        let first = PathBuf::from(r"C:\Temp\First");
        let second = PathBuf::from(r"C:\Temp\Second");
        let dropped = PathBuf::from(r"C:\Temp\Dropped");
        let mut entries = std::collections::BTreeMap::new();

        assert_eq!(
            upsert_pending_folder_cover_removal(&mut entries, &first, now, false, 2),
            DebounceQueueAction::Inserted
        );
        assert_eq!(
            upsert_pending_folder_cover_removal(&mut entries, &second, now, false, 2),
            DebounceQueueAction::Inserted
        );
        assert_eq!(
            upsert_pending_folder_cover_removal(&mut entries, &dropped, now, true, 2),
            DebounceQueueAction::Dropped
        );
        assert_eq!(
            upsert_pending_folder_cover_removal(
                &mut entries,
                &first,
                now - Duration::from_millis(1),
                true,
                2,
            ),
            DebounceQueueAction::Updated
        );
        assert_eq!(entries.len(), 2);
        assert!(!entries.contains_key(&dropped));
        assert_eq!(
            entries.get(&first),
            Some(&(now - Duration::from_millis(1), true))
        );
    }

    #[test]
    fn folder_cover_removal_runner_obeys_attempt_and_time_budgets() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(2);
        let mut entries = std::collections::BTreeMap::new();
        for name in ["A", "B", "C", "D"] {
            entries.insert(PathBuf::from(name), (now, false));
        }

        let clock_values = [
            now,
            now + Duration::from_millis(1),
            now + Duration::from_millis(2),
        ];
        let mut clock_index = 0;
        let attempts = run_folder_cover_removals_with_budget(
            &mut entries,
            Some(now),
            3,
            deadline,
            || {
                let value = clock_values[clock_index];
                clock_index += 1;
                value
            },
            |_, _| None,
        );

        assert_eq!(attempts, 2);
        assert_eq!(entries.len(), 2);

        let attempts = run_folder_cover_removals_with_budget(
            &mut entries,
            Some(now),
            1,
            now + Duration::from_secs(1),
            || now,
            |_, _| None,
        );
        assert_eq!(attempts, 1);
        assert_eq!(entries.len(), 1);
    }
}
