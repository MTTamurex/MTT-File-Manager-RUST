use crate::app::state::ImageViewerApp;
use crate::infrastructure::drive_watcher::DriveWatcherEvent;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

impl ImageViewerApp {
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

    fn register_changed_folder(changed_path: &Path, out: &mut HashSet<PathBuf>) {
        if crate::infrastructure::onedrive::fast_is_dir(changed_path) {
            out.insert(changed_path.to_path_buf());
        } else if let Some(parent) = changed_path.parent() {
            out.insert(parent.to_path_buf());
        }
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
        Self::register_changed_folder(&cleaned, folders_with_changed_contents);
        if let Some(parent) = cleaned.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.directory_cache.invalidate(&parent.to_path_buf());
                #[cfg(debug_assertions)]
                log::trace!(
                    "[FS-WATCH] CREATE: {:?}",
                    path.file_name().unwrap_or_default()
                );
                self.pending_auto_reload = true;
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
        self.invalidate_folder_cover_for_removed_path(&cleaned);
        Self::register_changed_folder(&cleaned, folders_with_changed_contents);

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
            self.directory_cache.invalidate(&current_path_buf);
            self.navigate_to_nearest_valid_ancestor();
            return;
        }

        if let Some(parent) = cleaned.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.directory_cache.invalidate(&parent.to_path_buf());
                self.directory_cache.invalidate_children(&cleaned);

                #[cfg(debug_assertions)]
                log::trace!(
                    "[FS-WATCH] DELETE: {:?}",
                    path.file_name().unwrap_or_default()
                );

                let path_to_remove = cleaned.clone();
                let removed_from_all = self
                    .all_items
                    .iter()
                    .position(|item| item.path == path_to_remove)
                    .map(|idx| {
                        self.all_items.remove(idx);
                        true
                    })
                    .unwrap_or(false);

                if removed_from_all {
                    let filtered: Vec<_> = self
                        .items
                        .iter()
                        .filter(|item| item.path != path_to_remove)
                        .cloned()
                        .collect();
                    self.items = Arc::new(filtered);
                    self.total_items = self.items.len();
                    #[cfg(debug_assertions)]
                    log::debug!("[FS-WATCH] SMART DELETE: Removed from UI without reload");

                    if let Some(selected) = self.selected_item {
                        if selected >= self.items.len() && !self.items.is_empty() {
                            self.selected_item = Some(self.items.len() - 1);
                        } else if self.items.is_empty() {
                            self.selected_item = None;
                            self.selected_file = None;
                        }
                    }

                    self.skip_next_auto_reload = true;
                }
            }
        }
    }

    fn handle_drive_modified_event(
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
        self.cache_manager.texture_cache.pop(&cleaned);
        self.cache_manager.failed_thumbnails.pop(&cleaned);
        crate::workers::thumbnail::clear_failure_cache(&cleaned);

        // Register parent folder as changed so its cover/preview caches
        // are invalidated when the modified file was the cover source.
        Self::register_changed_folder(&cleaned, folders_with_changed_contents);

        if let Some(ref selected) = self.selected_file {
            if selected.path == cleaned {
                self.metadata_cache.pop(&cleaned);
                self.last_metadata_path = None;
            }
        }

        // NOTE: MODIFY events mean file content/size/mtime changed, NOT that
        // files were added or removed. The directory listing (names, count) is
        // unchanged, so we do NOT invalidate the DirectoryCache or trigger a
        // full auto-reload. This prevents unnecessary disk rescans on FUSE/WinFsp
        // drivers (Cryptomator, VeraCrypt) that emit frequent MODIFY events
        // during internal operations.  The texture/thumbnail caches were already
        // evicted above, so the *visual* thumbnail will be refreshed lazily.
        // Directory listing metadata (size/mtime) is refreshed on next
        // navigation or manual reload (F5).
        #[cfg(debug_assertions)]
        if let Some(parent) = cleaned.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                log::trace!(
                    "[FS-WATCH] MODIFY (texture eviction only): {:?}",
                    path.file_name().unwrap_or_default()
                );
            }
        }
    }

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

        // Check if the CURRENT FOLDER was renamed.
        // If so, follow the rename to the new path instead of stranding the user.
        let old_norm = Self::normalize_for_match(&cleaned_old);
        if old_norm == current_path_norm {
            log::warn!(
                "[FS-WATCH] Current folder was RENAMED externally: {:?} → {:?}",
                cleaned_old, cleaned_new
            );
            self.directory_cache.invalidate(&cleaned_old);
            let new_path_str = cleaned_new.to_string_lossy().to_string();
            self.navigate_to(&new_path_str);
            return;
        }

        pending_disk_cache_invalidations.push(cleaned_old.clone());
        self.invalidate_folder_cover_for_removed_path(&cleaned_old);
        Self::register_changed_folder(&cleaned_old, folders_with_changed_contents);
        Self::register_changed_folder(&cleaned_new, folders_with_changed_contents);

        self.cache_manager.texture_cache.pop(&cleaned_old);
        self.cache_manager.texture_cache.pop(&cleaned_new);
        self.cache_manager.failed_thumbnails.pop(&cleaned_old);
        self.cache_manager.failed_thumbnails.pop(&cleaned_new);

        if let Some(parent) = cleaned_old.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.directory_cache.invalidate(&parent.to_path_buf());
                self.pending_auto_reload = true;
            }
        }
        if let Some(parent) = cleaned_new.parent() {
            let parent_norm = Self::normalize_for_match(parent);
            if parent_norm == current_path_norm {
                self.directory_cache.invalidate(&parent.to_path_buf());
                self.pending_auto_reload = true;
            }
        }
    }

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
                let current_path_pb = PathBuf::from(&self.navigation_state.current_path);
                if !self.navigation_state.is_computer_view
                    && !self.navigation_state.is_recycle_bin_view
                    && !current_path_pb.is_dir()
                {
                    log::warn!(
                        "[FS-WATCH] Flood: current folder vanished: {:?} — navigating up",
                        current_path_pb
                    );
                    self.directory_cache.invalidate(&current_path_pb);
                    self.navigate_to_nearest_valid_ancestor();
                    return;
                }

                if self.last_auto_reload.elapsed() > Duration::from_millis(flood_reload_cooldown_ms)
                {
                    self.directory_cache
                        .invalidate(&PathBuf::from(&self.navigation_state.current_path));
                    if !self.navigation_state.is_computer_view
                        && !self.navigation_state.is_recycle_bin_view
                    {
                        self.pending_auto_reload = true;
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
                            Self::register_changed_folder(&cleaned, folders_with_changed_contents);
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
                            Self::register_changed_folder(&cleaned, folders_with_changed_contents);
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
                            Self::register_changed_folder(
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
                            Self::register_changed_folder(
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
    ) {
        for folder_path in folders_with_changed_contents {
            self.disk_cache.remove_folder_preview_cache(&folder_path);
            self.disk_cache.remove_folder_cover(&folder_path);
            // Also evict the in-memory GPU texture so the stale preview
            // stops being rendered immediately (not just on LRU eviction).
            self.cache_manager.invalidate_folder_preview(&folder_path);
            self.scanned_folders.pop(&folder_path);
            let _ = self.cover_worker_sender.send(folder_path.clone());
        }
    }
}
