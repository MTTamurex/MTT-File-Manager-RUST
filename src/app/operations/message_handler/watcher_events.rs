use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) struct WatcherPerfMarks {
    pub(super) watcher_start: Instant,
    pub(super) drive_events_done: Instant,
    pub(super) auto_reload_done: Instant,
}

impl ImageViewerApp {
    pub(super) fn process_watcher_events_and_auto_reload(
        &mut self,
        current_path_norm: &str,
    ) -> WatcherPerfMarks {
        // PERFORMANCE: Filter by file name only - no filesystem I/O.
        // Hidden/system attribute filtering is already done in load_folder().
        // Previously called std::fs::metadata() here which caused synchronous
        // HDD reads on the UI thread for every watcher event.
        let internal_cache_root_norm =
            dirs::data_local_dir().map(|d| Self::normalize_for_match(&d.join("MTT-File-Manager")));
        let internal_cache_root_prefix = internal_cache_root_norm
            .as_ref()
            .map(|root| format!("{root}\\"));

        let should_ignore = |p: &Path| -> bool {
            let cleaned = Self::clean_path(p);
            let cleaned_norm = Self::normalize_for_match(&cleaned);
            let is_internal_cache_event = match (
                internal_cache_root_norm.as_ref(),
                internal_cache_root_prefix.as_ref(),
            ) {
                (Some(root), Some(prefix)) => {
                    cleaned_norm == *root || cleaned_norm.starts_with(prefix)
                }
                _ => false,
            };
            let name = p
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
        };

        // Drive-wide watcher (File Pilot optimization)
        // Check for pending watcher activation after startup delay
        let watcher_start = Instant::now();
        self.drive_watcher.check_pending_activation();

        let drive_events = self.drive_watcher.poll_events();
        let t_poll_done = Instant::now();
        let drive_watcher_active = !drive_events.is_empty();
        let drive_event_count = drive_events.len();

        // Handle DriveLost events immediately: the watcher thread detected that
        // the drive handle became invalid (drive was unmounted/disconnected).
        for event in &drive_events {
            if let crate::infrastructure::drive_watcher::DriveWatcherEvent::DriveLost(drive_root) =
                event
            {
                log::warn!("[FS-WATCH] DriveLost signal received for: {:?}", drive_root);
                self.drive_state.last_drive_refresh = Instant::now();
                self.reload_drive_list_async();

                // If user is browsing inside the lost drive, redirect immediately
                let drive_prefix = drive_root.to_string_lossy().to_string();
                if !self.navigation_state.is_computer_view
                    && !self.navigation_state.is_recycle_bin_view
                    && self.navigation_state.current_path.starts_with(&drive_prefix)
                {
                    log::warn!(
                        "[FS-WATCH] Current path '{}' is on lost drive, redirecting to Este Computador",
                        self.navigation_state.current_path
                    );
                    self.directory_cache.clear();
                    self.drive_watcher.cleanup_unused_watchers(None);
                    self.navigate_to_computer();
                    return WatcherPerfMarks {
                        watcher_start,
                        drive_events_done: Instant::now(),
                        auto_reload_done: Instant::now(),
                    };
                }
            }
        }

        // PERFORMANCE FIX: After long inactivity (OS sleep, display off), the watcher
        // thread accumulates many batches in the channel. Processing each event
        // individually does SQLite I/O (remove_cache_for_path = 6 SQL queries per DELETE).
        // With 500+ events, this blocks the UI thread for seconds.
        //
        // Strategy: If too many events accumulated, skip individual processing and
        // just trigger a simple folder reload. The reload will fetch fresh data from
        // disk, which is faster than processing hundreds of SQLite deletes.
        const MAX_EVENTS_INDIVIDUAL: usize = 50;
        const FLOOD_RELOAD_COOLDOWN_MS: u64 = 5000;
        let mut pending_disk_cache_invalidations: Vec<PathBuf> = Vec::new();
        let mut folders_with_changed_contents: HashSet<PathBuf> = HashSet::new();
        let register_changed_folder = |changed_path: &Path,
                                       out: &mut HashSet<PathBuf>| {
            // Some events arrive as "folder modified" (path is the folder),
            // others as "file changed" (path is the file). Handle both.
            if crate::infrastructure::onedrive::fast_is_dir(changed_path) {
                out.insert(changed_path.to_path_buf());
            } else if let Some(parent) = changed_path.parent() {
                out.insert(parent.to_path_buf());
            }
        };
        let flood_event_affects_current_listing =
            |event: &crate::infrastructure::drive_watcher::DriveWatcherEvent| -> bool {
                let path_affects = |p: &Path| -> bool {
                    let cleaned = Self::clean_path(p);
                    let cleaned_norm = Self::normalize_for_match(&cleaned);
                    if cleaned_norm == current_path_norm {
                        return true;
                    }

                    cleaned
                        .parent()
                        .map(|parent| Self::normalize_for_match(parent) == current_path_norm)
                        .unwrap_or(false)
                };

                match event {
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::Created(path)
                    | crate::infrastructure::drive_watcher::DriveWatcherEvent::Deleted(path)
                    | crate::infrastructure::drive_watcher::DriveWatcherEvent::Modified(path)
                    | crate::infrastructure::drive_watcher::DriveWatcherEvent::Unknown(path) => {
                        if should_ignore(path) {
                            return false;
                        }
                        path_affects(path)
                    }
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::Renamed(
                        old_path,
                        new_path,
                    ) => {
                        if should_ignore(old_path) && should_ignore(new_path) {
                            return false;
                        }
                        path_affects(old_path) || path_affects(new_path)
                    }
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::DriveLost(_) => true,
                }
            };

        if drive_events.len() > MAX_EVENTS_INDIVIDUAL {
            let affects_current_listing = drive_events
                .iter()
                .any(flood_event_affects_current_listing);

            if affects_current_listing {
                log::warn!(
                    "[FS-WATCH] Event flood detected: {} events (threshold {}). Direct impact on current folder, scheduling throttled reload.",
                    drive_events.len(),
                    MAX_EVENTS_INDIVIDUAL
                );

                // Keep flood handling bounded: don't reload repeatedly while the storm persists.
                if self.last_auto_reload.elapsed() > Duration::from_millis(FLOOD_RELOAD_COOLDOWN_MS)
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
        } else {
            // Events are pre-deduplicated and coalesced by the watcher thread (200ms batches,
            // max 500 unique events per batch). No rate-limiting needed here - the watcher
            // thread guarantees bounded event delivery even during OneDrive dehydration storms.
            for event in &drive_events {
                match event {
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::Created(path) => {
                        if should_ignore(path) {
                            continue;
                        }
                        let cleaned = Self::clean_path(path);
                        register_changed_folder(&cleaned, &mut folders_with_changed_contents);
                        if let Some(parent) = cleaned.parent() {
                            let parent_norm = Self::normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                // Only invalidate active folder cache to keep watcher handling O(1).
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
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::Deleted(path) => {
                        if should_ignore(path) {
                            continue;
                        }
                        let cleaned = Self::clean_path(path);
                        pending_disk_cache_invalidations.push(cleaned.clone());
                        self.invalidate_folder_cover_for_removed_path(&cleaned);
                        register_changed_folder(&cleaned, &mut folders_with_changed_contents);

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

                                // SMART DELETE: Remove da UI sem reload completo
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
                                    // Update items (Arc) - recreate without the deleted item
                                    let filtered: Vec<_> = self
                                        .items
                                        .iter()
                                        .filter(|item| item.path != path_to_remove)
                                        .cloned()
                                        .collect();
                                    self.items = Arc::new(filtered);
                                    self.total_items = self.items.len();
                                    #[cfg(debug_assertions)]
                                    log::debug!(
                                        "[FS-WATCH] SMART DELETE: Removed from UI without reload"
                                    );

                                    // Adjust selection if necessary
                                    if let Some(selected) = self.selected_item {
                                        if selected >= self.items.len() && !self.items.is_empty() {
                                            self.selected_item = Some(self.items.len() - 1);
                                        } else if self.items.is_empty() {
                                            self.selected_item = None;
                                            self.selected_file = None;
                                        }
                                    }

                                    // Prevent unnecessary reload - UI was already updated
                                    self.skip_next_auto_reload = true;
                                }
                            }
                        }
                    }
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::Modified(path) => {
                        if should_ignore(path) {
                            continue;
                        }
                        let cleaned = Self::clean_path(path);
                        self.cache_manager.texture_cache.pop(&cleaned);
                        self.cache_manager.failed_thumbnails.pop(&cleaned);
                        crate::workers::thumbnail::clear_failure_cache(&cleaned);

                        // Invalidate metadata cache if modified file is currently selected
                        if let Some(ref selected) = self.selected_file {
                            if selected.path == cleaned {
                                self.metadata_cache.pop(&cleaned);
                                self.last_metadata_path = None;
                            }
                        }

                        if let Some(parent) = cleaned.parent() {
                            let parent_norm = Self::normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                self.directory_cache.invalidate(&parent.to_path_buf());
                                #[cfg(debug_assertions)]
                                log::trace!(
                                    "[FS-WATCH] MODIFY: {:?}",
                                    path.file_name().unwrap_or_default()
                                );
                                self.pending_auto_reload = true;
                            }
                        }
                    }
                    crate::infrastructure::drive_watcher::DriveWatcherEvent::Renamed(
                        old_path,
                        new_path,
                    ) => {
                        if !should_ignore(old_path) || !should_ignore(new_path) {
                            let cleaned_old = Self::clean_path(old_path);
                            let cleaned_new = Self::clean_path(new_path);

                            pending_disk_cache_invalidations.push(cleaned_old.clone());
                            self.invalidate_folder_cover_for_removed_path(&cleaned_old);
                            register_changed_folder(&cleaned_old, &mut folders_with_changed_contents);
                            register_changed_folder(&cleaned_new, &mut folders_with_changed_contents);

                            // Invalidate caches for both paths
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
                    }
                    _ => {}
                }
            }
        }

        // Folder preview cache invalidation for content changes inside folders.
        // Needed so subfolder thumbnails/previews update when files are added/removed/renamed.
        for folder_path in folders_with_changed_contents {
            // Keep in-memory preview visible until a fresh one is ready to avoid
            // rapid fallback<->preview flicker in the details panel.
            self.disk_cache.remove_folder_preview_cache(&folder_path);
            self.disk_cache.remove_folder_cover(&folder_path);
            self.scanned_folders.pop(&folder_path);

            // Force a fresh cover discovery for this folder so changes from external apps
            // (Explorer, scripts, etc.) are reflected without manual refresh.
            let _ = self.cover_worker_sender.send(folder_path.clone());
        }

        let drive_events_done = Instant::now();
        if drive_events_done.duration_since(watcher_start).as_millis() > 50 {
            log::debug!(
                "[PERF-MSG] DriveWatcher: poll={}ms process={}ms events={}",
                t_poll_done.duration_since(watcher_start).as_millis(),
                drive_events_done.duration_since(t_poll_done).as_millis(),
                drive_event_count
            );
        }

        // LEGACY: Process notify-watcher events (kept for compatibility)
        // If drive-watcher already detected events, skip notify-watcher to avoid duplicates
        #[cfg(feature = "notify-watcher")]
        if !drive_watcher_active {
            // PERFORMANCE: Count events first to detect flood
            let mut legacy_events = Vec::new();
            while let Ok(event) = self.fs_event_receiver.try_recv() {
                legacy_events.push(event);
            }

            if legacy_events.len() > MAX_EVENTS_INDIVIDUAL {
                log::warn!(
                    "[FS-WATCH-LEGACY] Event flood detected: {} events. Triggering full reload.",
                    legacy_events.len()
                );
                self.directory_cache.clear();
                if !self.navigation_state.is_computer_view && !self.navigation_state.is_recycle_bin_view {
                    self.pending_auto_reload = true;
                }
            } else {
                for event in legacy_events {
                    match event {
                        Ok(evt) => {
                            let mut meaningful_change = false;

                            if matches!(evt.kind, notify::EventKind::Remove(_)) {
                                for path in &evt.paths {
                                    if should_ignore(path) {
                                        continue;
                                    }
                                    meaningful_change = true;

                                    let cleaned = Self::clean_path(path);
                                    if let Some(parent) = cleaned.parent() {
                                        self.directory_cache.invalidate(&parent.to_path_buf());
                                    }
                                    self.directory_cache.invalidate_children(&cleaned);
                                    #[cfg(debug_assertions)]
                                    log::trace!(
                                        "[FS-WATCH-LEGACY] REMOVE: {:?}",
                                        path.file_name().unwrap_or_default()
                                    );
                                    pending_disk_cache_invalidations.push(cleaned.clone());
                                }
                            }

                            for path in &evt.paths {
                                if should_ignore(path) {
                                    continue;
                                }
                                meaningful_change = true;

                                if let Some(parent) = path.parent() {
                                    let parent_norm = Self::normalize_for_match(parent);
                                    if parent_norm == current_path_norm {
                                        let cleaned = Self::clean_path(path);
                                        if let Some(cache_parent) = cleaned.parent() {
                                            self.directory_cache
                                                .invalidate(&cache_parent.to_path_buf());
                                        }
                                        #[cfg(debug_assertions)]
                                        log::trace!(
                                            "[FS] Direct subfolder modified: {:?}",
                                            cleaned.file_name()
                                        );
                                        self.disk_cache.remove_folder_preview_cache(&cleaned);
                                    }
                                }

                                if let Some(parent) = path.parent() {
                                    if let Some(grandparent) = parent.parent() {
                                        let grandparent_norm =
                                            Self::normalize_for_match(grandparent);
                                        if grandparent_norm == current_path_norm {
                                            let cleaned_parent = Self::clean_path(parent);
                                            if let Some(cache_parent) = cleaned_parent.parent() {
                                                self.directory_cache
                                                    .invalidate(&cache_parent.to_path_buf());
                                            }
                                            #[cfg(debug_assertions)]
                                            log::trace!(
                                                "[FS] File in subfolder modified, invalidating: {:?}",
                                                cleaned_parent.file_name()
                                            );
                                            self.disk_cache
                                                .remove_folder_preview_cache(&cleaned_parent);
                                        }
                                    }
                                }

                                let cleaned = Self::clean_path(path);
                                self.cache_manager.texture_cache.pop(&cleaned);
                                self.cache_manager.failed_thumbnails.pop(&cleaned);
                                crate::workers::thumbnail::clear_failure_cache(&cleaned);
                            }

                            if meaningful_change {
                                self.pending_auto_reload = true;
                            }
                        }
                        Err(_err) => {
                            #[cfg(debug_assertions)]
                            log::warn!("Erro de watch: {:?}", _err);
                        }
                    }
                }
            }
        }

        self.enqueue_disk_cache_invalidations(pending_disk_cache_invalidations);

        // Execute reload only when debounce allows
        // SUPPRESS auto-reload while file operations are in progress to prevent
        // screen flashing (watcher fires repeatedly as files grow during copy)
        // Skip auto-reload if smart delete already updated the UI
        if self.skip_next_auto_reload {
            self.skip_next_auto_reload = false;
            self.pending_auto_reload = false;
            #[cfg(debug_assertions)]
            log::debug!("[DEBUG] Skipping auto-reload - UI already updated by smart delete");
        }

        // NOTE: Inactivity recovery cooldown removed - no longer needed.
        // The DriveWatcher thread now coalesces and deduplicates events internally
        // (200ms batches, max 500 unique events per batch), so event floods from
        // OneDrive dehydration are absorbed before reaching the UI thread.
        if self.pending_auto_reload && self.file_operation_state.file_ops_in_progress == 0 && !self.is_loading_folder {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(theme::AUTO_RELOAD_MS) {
                #[cfg(debug_assertions)]
                log::debug!(
                    "[DEBUG] Checking auto-reload for path: '{}'",
                    self.navigation_state.current_path
                );
                // SKIP for special views (Recycle Bin/Computer) which are managed manually via events
                if self.navigation_state.is_recycle_bin_view || self.navigation_state.is_computer_view {
                    self.pending_auto_reload = false;
                } else {
                    #[cfg(debug_assertions)]
                    log::debug!(
                        "[DEBUG] Auto-reloading with force_refresh=false (watcher-triggered)."
                    );
                    // PERFORMANCE: Use force_refresh=false for watcher-triggered reloads.
                    // force_refresh=true clears ALL caches (textures, thumbnails, folder covers),
                    // empties the items list, and causes a white screen on HDD while rescanning.
                    // With false: directory_cache was already invalidated by watcher events above,
                    // so fresh data is loaded from disk, but texture/thumbnail caches are preserved.
                    // force_refresh=true is reserved for manual refresh (F5) only.
                    self.loaded_path.clear();
                    self.load_folder(false);
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }

        let auto_reload_done = Instant::now();
        WatcherPerfMarks {
            watcher_start,
            drive_events_done,
            auto_reload_done,
        }
    }
}

