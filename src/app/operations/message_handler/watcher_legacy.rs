use crate::app::state::ImageViewerApp;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn register_changed_folder(changed_path: &Path, out: &mut HashSet<PathBuf>) {
    if changed_path.extension().is_some() {
        if let Some(parent) = changed_path.parent() {
            out.insert(parent.to_path_buf());
        }
    } else {
        out.insert(changed_path.to_path_buf());
        if let Some(parent) = changed_path.parent() {
            out.insert(parent.to_path_buf());
        }
    }
}

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
        drive_watcher_active: bool,
        current_path_norm: &str,
        internal_cache_root_norm: Option<&str>,
        internal_cache_root_prefix: Option<&str>,
        max_events_individual: usize,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
    ) {
        // On USN filesystems, DriveWatcher is authoritative and we can skip notify.
        // On non-USN filesystems, keep notify processing as resilience backup.
        if drive_watcher_active && !self.watcher_fallback_polling {
            return;
        }

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
                            register_changed_folder(&cleaned, &mut folders_with_changed_contents);
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
                        if self.should_ignore_watcher_path(
                            path,
                            internal_cache_root_norm,
                            internal_cache_root_prefix,
                        ) {
                            continue;
                        }
                        meaningful_change = true;

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
                                self.disk_cache.remove_folder_preview_cache(&cleaned);
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
                                    self.disk_cache.remove_folder_preview_cache(&cleaned_parent);
                                }
                            }
                        }

                        let cleaned = Self::clean_path(path);
                        register_changed_folder(&cleaned, &mut folders_with_changed_contents);
                        let preserve_media_thumb = should_preserve_onedrive_media_thumbnail(&cleaned);
                        if !preserve_media_thumb {
                            self.cache_manager.texture_cache.pop(&cleaned);
                        }
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
                self.pending_auto_reload = true;
            }
        }

        for folder_path in folders_with_changed_contents {
            self.invalidate_folder_size_cache(&folder_path);
        }

        if has_more_events {
            self.ui_ctx.request_repaint();
        }
    }
}
