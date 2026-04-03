use crate::app::state::ImageViewerApp;
use crate::infrastructure::drive_watcher::DriveWatcherEvent;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Fast OneDrive check using path prefix only (no I/O).
/// Avoids `GetFileAttributesW` which can stall on cloud-only files.
/// Safe because OneDrive roots are resolved at startup via env vars.
fn is_onedrive_file(path: &Path) -> bool {
    crate::infrastructure::onedrive::is_onedrive_path(path)
}

impl ImageViewerApp {
    fn try_refresh_modified_file_entry_inline(&mut self, path: &Path, current_path_norm: &str) {
        let Some(parent) = path.parent() else {
            return;
        };

        if Self::normalize_for_match(parent) != current_path_norm {
            return;
        }

        // Skip blocking metadata() for OneDrive paths — sync/pin transitions
        // fire MODIFY events that don't change user-visible content.
        // The full folder reload will update size/mtime if needed.
        if is_onedrive_file(path) {
            return;
        }

        // FIX: Skip blocking std::fs::metadata() for paths on network/virtual drives.
        // GetFileInformationByHandle can block indefinitely on sleeping network shares,
        // VeraCrypt volumes, or USB drives entering standby.  The consistency probe
        // will pick up changes on the next cycle.
        if crate::infrastructure::io_priority::is_network_or_virtual(path) {
            return;
        }

        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };

        if !meta.is_file() {
            return;
        }

        let new_size = meta.len();
        let new_modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path_norm = Self::normalize_for_match(path);

        let mut touched = false;

        for item in self.all_items.iter_mut() {
            if item.path == path || Self::normalize_for_match(&item.path) == path_norm {
                item.size = new_size;
                item.modified = new_modified;
                touched = true;
                break;
            }
        }

        let items = Arc::make_mut(&mut self.items);
        for item in items.iter_mut() {
            if item.path == path || Self::normalize_for_match(&item.path) == path_norm {
                item.size = new_size;
                item.modified = new_modified;
                touched = true;
                break;
            }
        }

        if let Some(selected) = self.selected_file.as_mut() {
            if selected.path == path || Self::normalize_for_match(&selected.path) == path_norm {
                selected.size = new_size;
                selected.modified = new_modified;
                touched = true;
            }
        }

        if touched {
            self.ui_ctx.request_repaint();
            #[cfg(debug_assertions)]
            log::trace!(
                "[FS-WATCH] MODIFY inline metadata update: {:?} size={} modified={}",
                path.file_name().unwrap_or_default(),
                new_size,
                new_modified
            );
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

    fn path_affects_current_listing(current_path_norm: &str, path: &Path) -> bool {
        let cleaned = Self::clean_path(path);
        let cleaned_norm = Self::normalize_for_match(&cleaned);
        if cleaned_norm == current_path_norm {
            return true;
        }

        cleaned
            .parent()
            .map(|parent| Self::normalize_for_match(parent) == current_path_norm)
            .unwrap_or(false)
    }

    fn flood_event_affects_current_listing(
        &self,
        event: &DriveWatcherEvent,
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
    ) -> bool {
        match event {
            DriveWatcherEvent::Created(path)
            | DriveWatcherEvent::Deleted(path)
            | DriveWatcherEvent::Modified(path)
            | DriveWatcherEvent::Unknown(path) => {
                if self.should_ignore_watcher_path(
                    path,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                ) {
                    return false;
                }
                Self::path_affects_current_listing(current_path_norm, path)
            }
            DriveWatcherEvent::Renamed(old_path, new_path) => {
                if self.should_ignore_watcher_path(
                    old_path,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                ) && self.should_ignore_watcher_path(
                    new_path,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                ) {
                    return false;
                }
                Self::path_affects_current_listing(current_path_norm, old_path)
                    || Self::path_affects_current_listing(current_path_norm, new_path)
            }
            DriveWatcherEvent::DriveLost(_) => true,
        }
    }

    fn handle_drive_created_event(
        &mut self,
        path: &Path,
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        folders_with_changed_contents: &mut HashSet<PathBuf>,
    ) {
        if self.should_ignore_watcher_path(
            path,
            internal_cache_root_norm,
            internal_cache_root_prefix,
        ) {
            return;
        }

        let cleaned = Self::clean_path(path);
        crate::infrastructure::windows::file_flags::mark_recent_write_activity(&cleaned);
        self.register_changed_folder_for_path(&cleaned, folders_with_changed_contents);
        if let Some(parent) = cleaned.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.invalidate_directory_caches(parent);

                let ok = self.try_add_created_path_to_ui(&cleaned);
                log::info!(
                    "[FS-WATCH] CREATE: {:?} → smart_add={}",
                    path.file_name().unwrap_or_default(),
                    ok
                );
                if !ok {
                    self.request_watcher_auto_reload();
                }
            }
        }
    }

    fn handle_drive_deleted_event(
        &mut self,
        path: &Path,
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
        folders_with_changed_contents: &mut HashSet<PathBuf>,
    ) {
        if self.should_ignore_watcher_path(
            path,
            internal_cache_root_norm,
            internal_cache_root_prefix,
        ) {
            return;
        }

        let cleaned = Self::clean_path(path);
        pending_disk_cache_invalidations.push(cleaned.clone());
        self.register_changed_folder_for_path(&cleaned, folders_with_changed_contents);

        // Evict in-memory caches so a future file at the same path
        // won't inherit the deleted file's stale thumbnail texture.
        self.cache_manager.texture_cache.pop(&cleaned);
        self.cache_manager.failed_thumbnails.pop(&cleaned);

        // Check if the CURRENT FOLDER (or an ancestor) was deleted.
        // When that happens, the user is stranded in a non-existent path.
        let cleaned_norm = Self::normalize_for_match(&cleaned);
        let current_path_buf = PathBuf::from(&self.navigation_state.current_path);
        let current_is_deleted = cleaned_norm == current_path_norm;
        let ancestor_is_deleted = !current_is_deleted
            && current_path_buf
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&format!("{}\\" , cleaned_norm));

        if current_is_deleted || ancestor_is_deleted {
            log::warn!(
                "[FS-WATCH] Current folder (or ancestor) was DELETED externally: {:?}",
                cleaned
            );
            self.invalidate_directory_caches(&current_path_buf);
            self.navigate_to_nearest_valid_ancestor();
            return;
        }

        if let Some(parent) = cleaned.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.invalidate_directory_caches(parent);
                self.directory_cache.invalidate_children(&cleaned);

                #[cfg(debug_assertions)]
                log::trace!(
                    "[FS-WATCH] DELETE: {:?}",
                    path.file_name().unwrap_or_default()
                );

                if self.try_remove_deleted_path_from_ui(&cleaned) {
                    log::info!(
                        "[FS-WATCH] SMART DELETE OK: {:?}",
                        path.file_name().unwrap_or_default()
                    );
                    self.skip_next_auto_reload = true;
                } else {
                    log::info!(
                        "[FS-WATCH] SMART DELETE MISS → reload: {:?}",
                        path.file_name().unwrap_or_default()
                    );
                    self.request_watcher_auto_reload();
                }
            }
        }
    }

    fn handle_drive_modified_event(
        &mut self,
        path: &Path,
        _current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        folders_with_changed_contents: &mut HashSet<PathBuf>,
    ) {
        if self.should_ignore_watcher_path(
            path,
            internal_cache_root_norm,
            internal_cache_root_prefix,
        ) {
            return;
        }

        let cleaned = Self::clean_path(path);
    crate::infrastructure::windows::file_flags::mark_recent_write_activity(&cleaned);
        let is_onedrive = is_onedrive_file(&cleaned);

        self.try_refresh_modified_file_entry_inline(&cleaned, _current_path_norm);
        let preserve_media_thumb = is_onedrive
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(crate::infrastructure::windows::is_media_extension)
                .unwrap_or(false);

        if !preserve_media_thumb {
            self.cache_manager.texture_cache.pop(&cleaned);
        }
        self.cache_manager.failed_thumbnails.pop(&cleaned);

        // DON'T clear failure cache for files that are still being downloaded/written.
        // Otherwise: torrent writes piece → watcher fires MODIFY → failure cache cleared
        // → UI re-requests thumbnail/metadata → COM API opens file without FILE_SHARE_WRITE
        // → sharing violation kills the download. The cache will be cleared naturally
        // once the file is no longer unsafe to read (download completes).
        //
        // For OneDrive paths, skip the is_file_unsafe_to_read_fast check entirely.
        // OneDrive MODIFY events are sync/pin transitions, not active writes.
        // The metadata check inside unsafe_to_read does std::fs::metadata which
        // can stall on cloud-only files.
        let is_unsafe = if is_onedrive {
            false
        } else {
            crate::infrastructure::windows::file_flags::is_file_unsafe_to_read_fast(&cleaned)
        };
        if !is_unsafe {
            crate::workers::thumbnail::clear_failure_cache(&cleaned);
        }

        // Register parent folder as changed so its cover/preview caches
        // are invalidated when the modified file was the cover source.
        // Skip registration for ALL OneDrive MODIFY events: pin-state and sync
        // transitions fire MODIFY events that change file attributes (not content),
        // so the existing folder preview remains valid.  Re-generating it while
        // files are hydrating/dehydrating produces degraded icon-based previews.
        // Actual content changes from other devices arrive as CREATE/DELETE pairs
        // or are caught by the consistency probe.
        if !is_onedrive {
            self.register_changed_folder_for_path(&cleaned, folders_with_changed_contents);
        }

        if let Some(ref selected) = self.selected_file {
            if selected.path == cleaned {
                // Don't invalidate metadata cache for files actively being written.
                // Reuse the is_unsafe result to avoid a second blocking call.
                if !is_unsafe {
                    self.metadata_cache.pop(&cleaned);
                    self.last_metadata_path = None;
                }
            }
        }

        // NOTE: MODIFY events mean file content/size/mtime changed, NOT that
        // files were added or removed. The directory listing (names, count) is
        // unchanged, so we do NOT invalidate the DirectoryCache or trigger a
        // full auto-reload. This prevents unnecessary disk rescans on FUSE/WinFsp
        // drivers (Cryptomator, VeraCrypt) that emit frequent MODIFY events
        // during internal operations. For OneDrive media placeholders we keep
        // the last thumbnail (Explorer-like behavior) to avoid icon flicker.
        // File size/mtime are updated inline for visible entries to keep the
        // details panel and list columns accurate without manual reload.
        #[cfg(debug_assertions)]
        if let Some(parent) = cleaned.parent() {
            let current_path_norm = _current_path_norm;
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                log::trace!(
                    "[FS-WATCH] MODIFY (texture eviction only): {:?}",
                    path.file_name().unwrap_or_default()
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_drive_renamed_event(
        &mut self,
        old_path: &Path,
        new_path: &Path,
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
        folders_with_changed_contents: &mut HashSet<PathBuf>,
    ) {
        if self.should_ignore_watcher_path(
            old_path,
            internal_cache_root_norm,
            internal_cache_root_prefix,
        ) && self.should_ignore_watcher_path(
            new_path,
            internal_cache_root_norm,
            internal_cache_root_prefix,
        ) {
            return;
        }

        let cleaned_old = Self::clean_path(old_path);
        let cleaned_new = Self::clean_path(new_path);
        crate::infrastructure::windows::file_flags::mark_recent_write_activity(&cleaned_new);

        // Check if the CURRENT FOLDER was renamed.
        // If so, follow the rename to the new path instead of stranding the user.
        let old_norm = Self::normalize_for_match(&cleaned_old);
        if old_norm == current_path_norm {
            log::warn!(
                "[FS-WATCH] Current folder was RENAMED externally: {:?} → {:?}",
                cleaned_old, cleaned_new
            );
            self.invalidate_directory_caches(&cleaned_old);
            let new_path_str = cleaned_new.to_string_lossy().to_string();
            self.navigate_to(&new_path_str);
            return;
        }

        pending_disk_cache_invalidations.push(cleaned_old.clone());
        pending_disk_cache_invalidations.push(cleaned_new.clone());
        self.register_changed_folder_for_path(&cleaned_old, folders_with_changed_contents);
        self.register_changed_folder_for_path(&cleaned_new, folders_with_changed_contents);

        self.cache_manager.texture_cache.pop(&cleaned_old);
        self.cache_manager.texture_cache.pop(&cleaned_new);
        self.cache_manager.failed_thumbnails.pop(&cleaned_old);
        self.cache_manager.failed_thumbnails.pop(&cleaned_new);

        if let Some(parent) = cleaned_old.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.invalidate_directory_caches(parent);

                let ok = self.try_apply_rename_to_ui(&cleaned_old, &cleaned_new);
                log::info!(
                    "[FS-WATCH] RENAME: {:?} → {:?} smart={}",
                    cleaned_old.file_name().unwrap_or_default(),
                    cleaned_new.file_name().unwrap_or_default(),
                    ok
                );
                if !ok {
                    self.request_watcher_auto_reload();
                }
            }
        }
        if let Some(parent) = cleaned_new.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.invalidate_directory_caches(parent);
                if Self::normalize_for_match(&cleaned_old)
                    != Self::normalize_for_match(&cleaned_new)
                    && Self::normalize_for_match(cleaned_old.parent().unwrap_or(&cleaned_old))
                        != current_path_norm
                    && !self.try_add_created_path_to_ui(&cleaned_new)
                {
                    self.request_watcher_auto_reload();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn process_drive_events_batch(
        &mut self,
        drive_events: &[DriveWatcherEvent],
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        max_events_individual: usize,
        flood_reload_cooldown_ms: u64,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
        folders_with_changed_contents: &mut HashSet<PathBuf>,
    ) {
        if drive_events.len() > max_events_individual {
            let affects_current_listing = drive_events.iter().any(|event| {
                self.flood_event_affects_current_listing(
                    event,
                    current_path_norm,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                )
            });

            if affects_current_listing {
                log::warn!(
                    "[FS-WATCH] Event flood detected: {} events (threshold {}). Direct impact on current folder, scheduling throttled reload.",
                    drive_events.len(),
                    max_events_individual
                );

                // Check if the current folder was deleted/renamed during the flood.
                // This catches the case where a folder with many files is deleted
                // (generating >50 events) and the individual handler never runs.
                // Note: During our own file operations, this code is unreachable
                // because process_watcher_events_and_auto_reload() early-returns.
                //
                // We avoid calling is_dir() here because it invokes GetFileAttributesW
                // which can stall on OneDrive/network paths. Instead, check if any
                // of the flood events explicitly deleted/renamed the current folder.
                let current_path_pb = PathBuf::from(&self.navigation_state.current_path);
                let current_folder_deleted = drive_events.iter().any(|ev| match ev {
                    DriveWatcherEvent::Deleted(p) => {
                        Self::normalize_for_match(p) == current_path_norm
                    }
                    DriveWatcherEvent::Renamed(old, _) => {
                        Self::normalize_for_match(old) == current_path_norm
                    }
                    _ => false,
                });
                if !self.navigation_state.is_computer_view
                    && !self.navigation_state.is_recycle_bin_view
                    && current_folder_deleted
                {
                    log::warn!(
                        "[FS-WATCH] Flood: current folder vanished: {:?} — navigating up",
                        current_path_pb
                    );
                    self.invalidate_directory_caches(&current_path_pb);
                    self.navigate_to_nearest_valid_ancestor();
                    return;
                }

                if self.last_auto_reload.elapsed() > Duration::from_millis(flood_reload_cooldown_ms)
                {
                    let current_path = PathBuf::from(&self.navigation_state.current_path);
                    self.invalidate_directory_caches(&current_path);
                    if !self.navigation_state.is_computer_view
                        && !self.navigation_state.is_recycle_bin_view
                    {
                        self.request_watcher_auto_reload();
                    }
                } else {
                    #[cfg(debug_assertions)]
                    log::debug!(
                        "[FS-WATCH] Flood cooldown active ({}ms): skipping reload to avoid flicker",
                        self.last_auto_reload.elapsed().as_millis()
                    );
                }
            } else {
                #[cfg(debug_assertions)]
                log::debug!(
                    "[FS-WATCH] Event flood detected ({} events) with no direct impact on current folder listing. Skipping reload.",
                    drive_events.len()
                );
            }

            // Even during flood, collect affected parent folders so their
            // cover/preview caches are invalidated by apply_folder_content_change_invalidations.
            for event in drive_events {
                match event {
                    DriveWatcherEvent::Created(path)
                    | DriveWatcherEvent::Modified(path)
                    | DriveWatcherEvent::Unknown(path) => {
                        if !self.should_ignore_watcher_path(
                            path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        ) {
                            let cleaned = Self::clean_path(path);
                            self.register_changed_folder_for_path(&cleaned, folders_with_changed_contents);
                        }
                    }
                    DriveWatcherEvent::Deleted(path) => {
                        if !self.should_ignore_watcher_path(
                            path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        ) {
                            let cleaned = Self::clean_path(path);
                            pending_disk_cache_invalidations.push(cleaned.clone());
                            self.register_changed_folder_for_path(&cleaned, folders_with_changed_contents);
                        }
                    }
                    DriveWatcherEvent::Renamed(old_path, new_path) => {
                        if !self.should_ignore_watcher_path(
                            old_path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        ) {
                            let cleaned_old = Self::clean_path(old_path);
                            pending_disk_cache_invalidations.push(cleaned_old.clone());
                            self.register_changed_folder_for_path(
                                &cleaned_old,
                                folders_with_changed_contents,
                            );
                        }
                        if !self.should_ignore_watcher_path(
                            new_path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        ) {
                            let cleaned_new = Self::clean_path(new_path);
                            self.register_changed_folder_for_path(
                                &cleaned_new,
                                folders_with_changed_contents,
                            );
                        }
                    }
                    _ => {}
                }
            }
            return;
        }

        for event in drive_events {
            match event {
                DriveWatcherEvent::Created(path) => self.handle_drive_created_event(
                    path,
                    current_path_norm,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                    folders_with_changed_contents,
                ),
                DriveWatcherEvent::Deleted(path) => self.handle_drive_deleted_event(
                    path,
                    current_path_norm,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                    pending_disk_cache_invalidations,
                    folders_with_changed_contents,
                ),
                DriveWatcherEvent::Modified(path) => self.handle_drive_modified_event(
                    path,
                    current_path_norm,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                    folders_with_changed_contents,
                ),
                DriveWatcherEvent::Renamed(old_path, new_path) => self.handle_drive_renamed_event(
                    old_path,
                    new_path,
                    current_path_norm,
                    internal_cache_root_norm,
                    internal_cache_root_prefix,
                    pending_disk_cache_invalidations,
                    folders_with_changed_contents,
                ),
                _ => {}
            }
        }
    }

    pub(super) fn apply_folder_content_change_invalidations(
        &mut self,
        folders_with_changed_contents: HashSet<PathBuf>,
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
            Self::normalize_for_match(std::path::Path::new(&self.navigation_state.current_path));
        for folder_path in &folders_with_changed_contents {
            self.invalidate_folder_size_cache(folder_path);
            // Also evict the in-memory GPU texture so the stale preview
            // stops being rendered immediately (not just on LRU eviction).
            self.cache_manager.invalidate_folder_preview(folder_path);
            self.scanned_folders.pop(folder_path);
            // Keep folder listing cache coherent for future navigation.
            // This avoids opening a changed folder with stale cached entries
            // until the non-USN fallback probe runs (~30s).
            let folder_norm = Self::normalize_for_match(folder_path);
            if folder_norm != current_path_norm {
                self.directory_cache.invalidate(folder_path);
                self.clear_tab_cache_for_normalized_path(&folder_norm);
            }
            // Defer SQLite writes to the background invalidation worker to
            // avoid blocking the UI thread during watcher bursts.
            pending_disk_cache_invalidations.push(folder_path.clone());
            let _ = self.cover_worker_sender.send(folder_path.clone());

            // Also invalidate the PARENT directory's cache.
            //
            // When files change inside folder B, B's own mtime is updated by
            // Windows but the cached listing of B's parent (A) still contains
            // B's old mtime.  Without this invalidation, navigating back to A
            // would serve the stale cache (A's own mtime didn't change, so the
            // fast-path mtime validation passes) and sorting by date would show
            // B in the wrong position until a manual F5 refresh.
            if let Some(parent) = folder_path.parent() {
                let parent_buf = parent.to_path_buf();
                if !folders_with_changed_contents.contains(&parent_buf) {
                    let parent_norm = Self::normalize_for_match(parent);
                    self.directory_cache.invalidate(&parent_buf);
                    self.clear_tab_cache_for_normalized_path(&parent_norm);
                }
            }
        }

        // Clear stale folder_cover on items whose folder had content changes.
        // This prevents the UI from loading thumbnails for a cover file that
        // may no longer exist, and ensures the cover_worker result is the
        // single source of truth when it arrives.
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
            let items = std::sync::Arc::make_mut(&mut self.items);
            for item in items.iter_mut() {
                if item.is_dir
                    && item.folder_cover.is_some()
                    && folders_with_changed_contents.contains(&item.path)
                {
                    item.folder_cover = None;
                }
            }
        }

        // FIX: Refresh `modified` timestamp for folders visible in the current
        // listing whose contents changed.  Without this, sorting by modification
        // date would not reorder the folder until a manual F5 refresh because
        // the watcher events fire for files *inside* the folder, leaving the
        // folder item's cached `modified` field stale.
        //
        // DEBOUNCED approach to avoid UI overload during sustained writes
        // (downloads, torrents, builds, etc.):
        //
        // Instead of reading metadata + re-sorting immediately (which can fire
        // dozens of times per second during a download), we schedule a deferred
        // recheck with a sliding-window debounce.  Each new event for the same
        // folder pushes the deadline forward so rapid-fire events coalesce into
        // a single metadata read + re-sort once the burst settles.
        //
        // The recheck delay (2s) gives Windows enough time to flush the
        // directory's LastWriteTime after all file handles are closed.
        let mut scheduled_any = false;
        for folder_path in &folders_with_changed_contents {
            // Skip metadata() calls on OneDrive and network/virtual drives.
            if is_onedrive_file(folder_path) {
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

            // Only schedule for folders that are visible in the current listing.
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

            // Sliding-window debounce: if already pending, push the deadline
            // forward instead of adding a duplicate.  This coalesces rapid
            // events (e.g. download writing every 100ms) into one recheck.
            let recheck_at = std::time::Instant::now() + Duration::from_secs(2);
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
                // Cap the pending list to prevent unbounded growth under a flood
                // of watcher events hitting many distinct folders.
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

        // Request a repaint around the time the earliest recheck is due,
        // so the deferred handler runs even when the app is idle.
        if scheduled_any {
            self.ui_ctx
                .request_repaint_after(Duration::from_millis(2500));
        }
    }

    /// Process deferred folder mtime rechecks.
    ///
    /// Called from the main watcher event loop.  Re-reads modification timestamps
    /// from the filesystem for folders that were flagged earlier but whose mtime
    /// hadn't been updated yet (Windows lazy-write delay).
    ///
    /// Uses a global cooldown (`last_folder_mtime_sort`) to ensure at most one
    /// re-sort every 3 seconds, preventing UI thrashing during sustained write
    /// bursts (downloads, torrent seeding, build output, etc.).
    pub(super) fn process_pending_folder_mtime_rechecks(&mut self) {
        if self.pending_folder_mtime_recheck.is_empty() {
            return;
        }

        let now = std::time::Instant::now();

        // Global cooldown: don't re-sort more often than every 3 seconds.
        // This prevents dozens of re-sorts during sustained write activity
        // (e.g. downloading a large file, torrent seeding, compiler output).
        const MTIME_SORT_COOLDOWN: Duration = Duration::from_secs(3);
        let cooldown_remaining = MTIME_SORT_COOLDOWN
            .checked_sub(now.duration_since(self.last_folder_mtime_sort))
            .unwrap_or(Duration::ZERO);
        if cooldown_remaining > Duration::ZERO {
            // Schedule repaint for when cooldown expires so we don't miss it.
            self.ui_ctx
                .request_repaint_after(cooldown_remaining + Duration::from_millis(50));
            return;
        }

        // Collect paths whose recheck time has arrived.
        let due_entries: Vec<PathBuf> = self
            .pending_folder_mtime_recheck
            .iter()
            .filter(|(_, recheck_at)| now >= *recheck_at)
            .map(|(p, _)| p.clone())
            .collect();

        if due_entries.is_empty() {
            // There are pending entries but none due yet.
            // Schedule repaint for the earliest due time.
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

        // Remove due entries from the pending list.
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

            // Find the item's current mtime for logging.
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

            // Invalidate the directory cache for the current listing so that
            // navigating away and back won't serve stale entries with old
            // folder mtimes.  The live `all_items` already have the correct
            // timestamps, but the cache snapshot was taken before the update.
            let current_path_buf =
                PathBuf::from(&self.navigation_state.current_path);
            self.directory_cache.invalidate(&current_path_buf);
        }

        // If there are still pending rechecks, schedule repaint for next due time.
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
