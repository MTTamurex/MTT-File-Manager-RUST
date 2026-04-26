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

#[cfg(feature = "notify-watcher")]
fn watcher_path_matches_current_folder(path: &std::path::Path, current_path_norm: &str) -> bool {
    let cleaned = ImageViewerApp::clean_path(path);
    ImageViewerApp::normalize_for_match(&cleaned) == current_path_norm
}

#[cfg(feature = "notify-watcher")]
fn notify_event_removes_current_folder(event: &notify::Event, current_path_norm: &str) -> bool {
    match &event.kind {
        notify::EventKind::Remove(_) => event
            .paths
            .iter()
            .any(|path| watcher_path_matches_current_folder(path, current_path_norm)),
        notify::EventKind::Modify(ModifyKind::Name(rename_mode)) => match rename_mode {
            RenameMode::From => event
                .paths
                .first()
                .is_some_and(|path| watcher_path_matches_current_folder(path, current_path_norm)),
            RenameMode::Both | RenameMode::Any => {
                let old_matches = event.paths.first().is_some_and(|path| {
                    watcher_path_matches_current_folder(path, current_path_norm)
                });
                let new_matches = event.paths.get(1).is_some_and(|path| {
                    watcher_path_matches_current_folder(path, current_path_norm)
                });
                old_matches && !new_matches
            }
            RenameMode::To | RenameMode::Other => false,
        },
        _ => false,
    }
}

#[cfg(feature = "notify-watcher")]
fn notify_error_implies_current_folder_removed(
    error: &notify::Error,
    current_path_norm: &str,
) -> bool {
    error
        .paths
        .iter()
        .any(|path| watcher_path_matches_current_folder(path, current_path_norm))
        || (error.paths.is_empty() && matches!(error.kind, notify::ErrorKind::PathNotFound))
}

#[cfg(feature = "notify-watcher")]
fn notify_event_is_name_change(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        notify::EventKind::Modify(ModifyKind::Name(
            RenameMode::Any
                | RenameMode::Both
                | RenameMode::From
                | RenameMode::To
                | RenameMode::Other
        ))
    )
}

impl ImageViewerApp {
    #[cfg(feature = "notify-watcher")]
    fn navigate_after_current_folder_removed_by_notify(&mut self, reason: &str) {
        log::warn!("{}: {}", reason, self.navigation_state.current_path);
        self.pending_auto_reload = false;
        self.skip_next_auto_reload = false;
        self.navigate_to_nearest_valid_ancestor();
    }

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
                    let current_folder_removed =
                        notify_event_removes_current_folder(&evt, current_path_norm);
                    let is_name_change = notify_event_is_name_change(&evt);

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
                            self.register_changed_folder_for_path(
                                &cleaned,
                                &mut folders_with_changed_contents,
                            );
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

                        if current_folder_removed {
                            let affected_folders: Vec<PathBuf> = evt
                                .paths
                                .iter()
                                .map(|path| Self::clean_path(path))
                                .filter_map(|path| path.parent().map(|parent| parent.to_path_buf()))
                                .collect();
                            let affected_refs: Vec<&PathBuf> = affected_folders.iter().collect();
                            self.reload_inactive_panel_if_matches(&affected_refs);
                            self.navigate_after_current_folder_removed_by_notify(
                                "[FS-WATCH-LEGACY] Current folder was removed externally",
                            );
                            return;
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
                            crate::infrastructure::windows::file_flags::mark_recent_write_activity(
                                &cleaned,
                            );
                            self.register_changed_folder_for_path(
                                &cleaned,
                                &mut folders_with_changed_contents,
                            );

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

                    if is_name_change && evt.paths.len() >= 2 {
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
                            crate::infrastructure::windows::file_flags::mark_recent_write_activity(
                                &cleaned_new,
                            );
                            self.register_changed_folder_for_path(
                                &cleaned_old,
                                &mut folders_with_changed_contents,
                            );
                            self.register_changed_folder_for_path(
                                &cleaned_new,
                                &mut folders_with_changed_contents,
                            );
                            self.apply_rename_to_inactive_panel_if_affected(
                                &cleaned_old,
                                &cleaned_new,
                            );

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
                                .map(|parent| {
                                    Self::normalize_for_match(parent) == current_path_norm
                                })
                                .unwrap_or(false);
                            let new_in_current = cleaned_new
                                .parent()
                                .map(|parent| {
                                    Self::normalize_for_match(parent) == current_path_norm
                                })
                                .unwrap_or(false);

                            if old_in_current || new_in_current {
                                if !self.try_apply_rename_to_ui(&cleaned_old, &cleaned_new) {
                                    needs_reload = true;
                                }
                            }

                            if current_folder_removed {
                                let mut affected_folders =
                                    vec![cleaned_old.clone(), cleaned_new.clone()];
                                if let Some(parent) = cleaned_old.parent() {
                                    affected_folders.push(parent.to_path_buf());
                                }
                                if let Some(parent) = cleaned_new.parent() {
                                    affected_folders.push(parent.to_path_buf());
                                }
                                let affected_refs: Vec<&PathBuf> =
                                    affected_folders.iter().collect();
                                self.reload_inactive_panel_if_matches(&affected_refs);
                                self.navigate_after_current_folder_removed_by_notify(
                                    "[FS-WATCH-LEGACY] Current folder was renamed or moved externally",
                                );
                                return;
                            }
                        }
                    }

                    if is_name_change {
                        let affected_folders: Vec<PathBuf> = evt
                            .paths
                            .iter()
                            .filter(|path| {
                                !self.should_ignore_watcher_path(
                                    path,
                                    internal_cache_root_norm,
                                    internal_cache_root_prefix,
                                )
                            })
                            .map(|path| Self::clean_path(path))
                            .filter_map(|path| path.parent().map(|parent| parent.to_path_buf()))
                            .collect();

                        if !affected_folders.is_empty() {
                            for folder in &affected_folders {
                                self.invalidate_directory_caches(folder);
                            }

                            if affected_folders.iter().any(|folder| {
                                Self::normalize_for_match(folder) == current_path_norm
                            }) {
                                needs_reload = true;
                            }

                            let affected_refs: Vec<&PathBuf> = affected_folders.iter().collect();
                            self.reload_inactive_panel_if_matches(&affected_refs);
                        }
                    }

                    if current_folder_removed && is_name_change {
                        let affected_folders: Vec<PathBuf> = evt
                            .paths
                            .iter()
                            .map(|path| Self::clean_path(path))
                            .filter_map(|path| path.parent().map(|parent| parent.to_path_buf()))
                            .collect();
                        let affected_refs: Vec<&PathBuf> = affected_folders.iter().collect();
                        self.reload_inactive_panel_if_matches(&affected_refs);
                        self.navigate_after_current_folder_removed_by_notify(
                            "[FS-WATCH-LEGACY] Current folder was renamed or moved externally",
                        );
                        return;
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
                        if is_name_change {
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
                        crate::infrastructure::windows::file_flags::mark_recent_write_activity(
                            &cleaned,
                        );
                        self.register_changed_folder_for_path(
                            &cleaned,
                            &mut folders_with_changed_contents,
                        );
                        let preserve_media_thumb =
                            should_preserve_onedrive_media_thumbnail(&cleaned);
                        if !preserve_media_thumb {
                            self.cache_manager.texture_cache.pop(&cleaned);
                        }
                        self.cache_manager.failed_thumbnails.pop(&cleaned);

                        // DON'T clear failure cache for files still being written
                        // (active downloads).
                        // Uses _fast variant to avoid blocking UI thread.
                        if !crate::infrastructure::windows::file_flags::is_file_unsafe_to_read_fast(
                            &cleaned,
                        ) {
                            crate::workers::thumbnail::clear_failure_cache(&cleaned);
                        }
                    }

                    if meaningful_change {
                        if needs_reload {
                            self.request_watcher_auto_reload();
                        }
                    }
                }
                Err(err) => {
                    #[cfg(debug_assertions)]
                    log::warn!("Erro de watch: {:?}", err);

                    if notify_error_implies_current_folder_removed(&err, current_path_norm) {
                        let current_path = PathBuf::from(&self.navigation_state.current_path);
                        if let Some(parent) = current_path.parent() {
                            let parent_path = parent.to_path_buf();
                            self.reload_inactive_panel_if_matches(&[&parent_path]);
                        }
                        self.navigate_after_current_folder_removed_by_notify(
                            "[FS-WATCH-LEGACY] Watcher reported missing current folder",
                        );
                        return;
                    }
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
            let affected_refs: Vec<&PathBuf> = folders_with_changed_contents.iter().collect();
            self.reload_inactive_panel_if_matches(&affected_refs);
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

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{ModifyKind, RemoveKind, RenameMode};
    use notify::{Event, EventKind};
    use std::path::{Path, PathBuf};

    fn normalized(path: &str) -> String {
        ImageViewerApp::normalize_for_match(Path::new(path))
    }

    #[test]
    fn detects_current_folder_remove_event() {
        let current = normalized(r"C:\Temp\Gone");
        let event = Event::new(EventKind::Remove(RemoveKind::Folder))
            .add_path(PathBuf::from(r"\\?\C:\Temp\Gone"));

        assert!(notify_event_removes_current_folder(&event, &current));
    }

    #[test]
    fn ignores_removed_child_inside_current_folder() {
        let current = normalized(r"C:\Temp\Gone");
        let event = Event::new(EventKind::Remove(RemoveKind::File))
            .add_path(PathBuf::from(r"C:\Temp\Gone\child.txt"));

        assert!(!notify_event_removes_current_folder(&event, &current));
    }

    #[test]
    fn detects_current_folder_renamed_away() {
        let current = normalized(r"C:\Temp\Gone");
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(PathBuf::from(r"C:\Temp\Gone"))
            .add_path(PathBuf::from(r"C:\Temp\Renamed"));

        assert!(notify_event_removes_current_folder(&event, &current));
    }

    #[test]
    fn detects_single_path_current_folder_rename_from_event() {
        let current = normalized(r"C:\Temp\Gone");
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
            .add_path(PathBuf::from(r"C:\Temp\Gone"));

        assert!(notify_event_removes_current_folder(&event, &current));
    }

    #[test]
    fn ignores_rename_into_current_folder_path() {
        let current = normalized(r"C:\Temp\Gone");
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(PathBuf::from(r"C:\Temp\Other"))
            .add_path(PathBuf::from(r"C:\Temp\Gone"));

        assert!(!notify_event_removes_current_folder(&event, &current));
    }

    #[test]
    fn treats_current_folder_path_not_found_error_as_removed() {
        let current = normalized(r"C:\Temp\Gone");
        let error = notify::Error::path_not_found();

        assert!(notify_error_implies_current_folder_removed(
            &error, &current
        ));
    }

    #[test]
    fn treats_watcher_error_for_current_folder_as_removed() {
        let current = normalized(r"C:\Temp\Gone");
        let error =
            notify::Error::generic("watch failed").add_path(PathBuf::from(r"\\?\C:\Temp\Gone"));

        assert!(notify_error_implies_current_folder_removed(
            &error, &current
        ));
    }

    #[test]
    fn treats_other_name_modify_event_as_name_change() {
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Other)))
            .add_path(PathBuf::from(r"C:\Temp\New folder"));

        assert!(notify_event_is_name_change(&event));
    }
}
