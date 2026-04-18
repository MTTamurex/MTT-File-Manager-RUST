use crate::app::state::ImageViewerApp;
use notify::event::{ModifyKind, RenameMode};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn should_preserve_onedrive_media_thumbnail(path: &std::path::Path) -> bool {
    if !crate::infrastructure::onedrive::is_onedrive_path(path)
        && !crate::infrastructure::onedrive::path_has_cloud_attributes(path)
    {
        return false;
    }

    path.extension()
        .and_then(|e| e.to_str())
        .map(crate::infrastructure::windows::is_media_extension)
        .unwrap_or(false)
}

impl ImageViewerApp {
    #[cfg(feature = "notify-watcher")]
    pub(super) fn process_legacy_notify_events(
        &mut self,
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        max_events_individual: usize,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
    ) {
        let budget = if self.layout.saved_is_minimized {
            Duration::from_millis(1)
        } else if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(2)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(3)
        } else {
            Duration::from_millis(5)
        };

        let start = Instant::now();
        let mut processed_events = 0usize;
        let mut has_more_events = false;

        let mut folders_with_changed_contents: HashSet<PathBuf> = HashSet::new();

        while processed_events < max_events_individual {
            if start.elapsed() >= budget {
                has_more_events = true;
                break;
            }

            let event = match self.fs_event_receiver.try_recv() {
                Ok(event) => event,
                Err(_) => break,
            };
            processed_events += 1;

            match event {
                Ok(evt) => {
                    let mut meaningful_change = false;
                    let mut needs_reload = false;

                    if matches!(evt.kind, notify::EventKind::Remove(_)) {
                        for path in &evt.paths {
                            if self.should_ignore_watcher_path(
                                path,
                                internal_cache_root_norm,
                                internal_cache_root_prefix,
                            ) {
                                continue;
                            }
                            meaningful_change = true;

                            let cleaned = Self::clean_path(path);
                            self.register_changed_folder_for_path(&cleaned, &mut folders_with_changed_contents);
                            if let Some(parent) = cleaned.parent() {
                                self.invalidate_directory_caches(parent);

                                let parent_norm = Self::normalize_for_match(parent);
                                if parent_norm == current_path_norm {
                                    if self.try_remove_deleted_path_from_ui(&cleaned) {
                                        #[cfg(debug_assertions)]
                                        log::debug!(
                                            "[FS-WATCH-LEGACY] SMART DELETE: Removed from UI without reload"
                                        );
                                        self.skip_next_auto_reload = true;
                                    }
                                }
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

                    if matches!(evt.kind, notify::EventKind::Create(_)) {
                        for path in &evt.paths {
                            if self.should_ignore_watcher_path(
                                path,
                                internal_cache_root_norm,
                                internal_cache_root_prefix,
                            ) {
                                continue;
                            }

                            meaningful_change = true;
                            let cleaned = Self::clean_path(path);
                            crate::infrastructure::windows::file_flags::mark_recent_write_activity(&cleaned);
                            self.register_changed_folder_for_path(&cleaned, &mut folders_with_changed_contents);

                            if let Some(parent) = cleaned.parent() {
                                self.invalidate_directory_caches(parent);

                                let parent_norm = Self::normalize_for_match(parent);
                                if parent_norm == current_path_norm {
                                    if !self.try_add_created_path_to_ui(&cleaned) {
                                        needs_reload = true;
                                    }
                                }
                            }

                            pending_disk_cache_invalidations.push(cleaned.clone());
                        }
                    }

                    if matches!(
                        evt.kind,
                        notify::EventKind::Modify(ModifyKind::Name(
                            RenameMode::Any | RenameMode::Both | RenameMode::From | RenameMode::To
                        ))
                    ) && evt.paths.len() >= 2
                    {
                        let old_path = &evt.paths[0];
                        let new_path = &evt.paths[1];

                        let ignore_old = self.should_ignore_watcher_path(
                            old_path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        );
                        let ignore_new = self.should_ignore_watcher_path(
                            new_path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        );

                        if !(ignore_old && ignore_new) {
                            meaningful_change = true;

                            let cleaned_old = Self::clean_path(old_path);
                            let cleaned_new = Self::clean_path(new_path);
                            crate::infrastructure::windows::file_flags::mark_recent_write_activity(&cleaned_new);
                            self.register_changed_folder_for_path(&cleaned_old, &mut folders_with_changed_contents);
                            self.register_changed_folder_for_path(&cleaned_new, &mut folders_with_changed_contents);

                            pending_disk_cache_invalidations.push(cleaned_old.clone());
                            pending_disk_cache_invalidations.push(cleaned_new.clone());

                            if let Some(parent) = cleaned_old.parent() {
                                self.invalidate_directory_caches(parent);
                            }
                            if let Some(parent) = cleaned_new.parent() {
                                self.invalidate_directory_caches(parent);
                            }

                            let old_in_current = cleaned_old
                                .parent()
                                .map(|parent| Self::normalize_for_match(parent) == current_path_norm)
                                .unwrap_or(false);
                            let new_in_current = cleaned_new
                                .parent()
                                .map(|parent| Self::normalize_for_match(parent) == current_path_norm)
                                .unwrap_or(false);

                            if old_in_current || new_in_current {
                                if !self.try_apply_rename_to_ui(&cleaned_old, &cleaned_new) {
                                    needs_reload = true;
                                }
                            }
                        }
                    }

                    for path in &evt.paths {
                        if self.should_ignore_watcher_path(
                            path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        ) {
                            continue;
                        }
                        meaningful_change = true;

                        if matches!(evt.kind, notify::EventKind::Create(_)) {
                            continue;
                        }
                        if matches!(
                            evt.kind,
                            notify::EventKind::Modify(ModifyKind::Name(
                                RenameMode::Any | RenameMode::Both | RenameMode::From | RenameMode::To
                            ))
                        ) {
                            continue;
                        }

                        if let Some(parent) = path.parent() {
                            let parent_norm = Self::normalize_for_match(parent);
                            if parent_norm == current_path_norm {
                                let cleaned = Self::clean_path(path);
                                if let Some(cache_parent) = cleaned.parent() {
                                    self.directory_cache.invalidate(&cache_parent.to_path_buf());
                                }
                                #[cfg(debug_assertions)]
                                log::trace!(
                                    "[FS] Direct subfolder modified: {:?}",
                                    cleaned.file_name()
                                );
                                // PERF FIX (M-6): Defer SQLite writer lock to background
                                // instead of blocking UI thread during watcher events.
                                pending_disk_cache_invalidations.push(cleaned);
                            }
                        }

                        if let Some(parent) = path.parent() {
                            if let Some(grandparent) = parent.parent() {
                                let grandparent_norm = Self::normalize_for_match(grandparent);
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
                                    // PERF FIX (M-6): Defer SQLite writer lock to background.
                                    pending_disk_cache_invalidations.push(cleaned_parent);
                                }
                            }
                        }

                        let cleaned = Self::clean_path(path);
                        crate::infrastructure::windows::file_flags::mark_recent_write_activity(&cleaned);
                        self.register_changed_folder_for_path(&cleaned, &mut folders_with_changed_contents);
                        let preserve_media_thumb = should_preserve_onedrive_media_thumbnail(&cleaned);
                        if !preserve_media_thumb {
                            self.cache_manager.texture_cache.pop(&cleaned);
                        }
                        self.cache_manager.failed_thumbnails.pop(&cleaned);

                        // DON'T clear failure cache for files still being written
                        // (active downloads).
                        // Uses _fast variant to avoid blocking UI thread.
                        if !crate::infrastructure::windows::file_flags::is_file_unsafe_to_read_fast(&cleaned) {
                            crate::workers::thumbnail::clear_failure_cache(&cleaned);
                        }
                    }

                    if meaningful_change {
                        if needs_reload {
                            self.request_watcher_auto_reload();
                        }
                    }
                }
                Err(_err) => {
                    #[cfg(debug_assertions)]
                    log::warn!("Erro de watch: {:?}", _err);
                }
            }
        }

        if processed_events >= max_events_individual {
            has_more_events = true;
            log::warn!(
                "[FS-WATCH-LEGACY] Event flood detected (processed {} in one frame). Triggering full reload.",
                processed_events
            );
            self.directory_cache.clear();
            if !self.navigation_state.is_computer_view && !self.navigation_state.is_recycle_bin_view
            {
                let current_path = PathBuf::from(&self.navigation_state.current_path);
                self.invalidate_folder_size_cache(current_path.as_path());
                self.request_watcher_auto_reload();
            }
        }

        for folder_path in &folders_with_changed_contents {
            self.invalidate_folder_size_cache(folder_path);
            self.sidebar_tree.clear_children(folder_path);
        }

        if !folders_with_changed_contents.is_empty() {
            self.apply_folder_content_change_invalidations(
                folders_with_changed_contents,
                pending_disk_cache_invalidations,
            );
        }

        if has_more_events {
            self.ui_ctx.request_repaint();
        }
    }
}
