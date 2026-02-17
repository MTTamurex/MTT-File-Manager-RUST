use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) struct WatcherPerfMarks {
    pub(super) watcher_start: Instant,
    pub(super) drive_events_done: Instant,
    pub(super) auto_reload_done: Instant,
}

impl ImageViewerApp {
    /// Returns the poll interval based on RDCW reliability verdict.
    /// - Confirmed unreliable -> fast polling on local USN filesystems
    /// - Non-USN/virtual filesystems -> conservative polling to avoid UI stalls
    fn fallback_poll_interval(&self, item_count: usize) -> Duration {
        let is_non_usn_fs = self
            .watcher_fallback_fs
            .as_deref()
            .map(|fs| !(fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS")))
            .unwrap_or(false);

        let drive_letter =
            crate::infrastructure::windows::extract_drive_letter(std::path::Path::new(
                &self.navigation_state.current_path,
            ));
        let known_bad = drive_letter
            .map(|dl| self.rdcw_unreliable_drives.get(&dl).copied().unwrap_or(false))
            .unwrap_or(false);

        if known_bad {
            if is_non_usn_fs {
                if item_count <= 2_000 {
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(45)
                }
            } else if item_count <= 300 {
                Duration::from_secs(3)
            } else if item_count <= 2_000 {
                Duration::from_secs(6)
            } else if item_count <= 8_000 {
                Duration::from_secs(10)
            } else {
                Duration::from_secs(15)
            }
        } else if is_non_usn_fs {
            Duration::from_secs(45)
        } else {
            Duration::from_secs(30)
        }
    }

    fn compute_entries_signature(entries: &[FileEntry]) -> u64 {
        let mut xor_acc = 0u64;
        let mut sum_acc = 0u64;
        let mut bytes_acc = 0u64;

        for entry in entries {
            let mut hasher = DefaultHasher::new();
            entry.name.hash(&mut hasher);
            entry.is_dir.hash(&mut hasher);
            entry.size.hash(&mut hasher);
            entry.modified.hash(&mut hasher);
            let entry_hash = hasher.finish();

            xor_acc ^= entry_hash;
            sum_acc = sum_acc.wrapping_add(entry_hash);
            bytes_acc = bytes_acc.wrapping_add(entry.size);
        }

        let mut final_hasher = DefaultHasher::new();
        entries.len().hash(&mut final_hasher);
        xor_acc.hash(&mut final_hasher);
        sum_acc.hash(&mut final_hasher);
        bytes_acc.hash(&mut final_hasher);
        final_hasher.finish()
    }

    fn maybe_poll_non_usn_consistency(
        &mut self,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
    ) {
        if !self.watcher_fallback_polling
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
            || self.is_loading_folder
            || self.file_operation_state.file_ops_in_progress > 0
            || self.layout.saved_is_minimized
        {
            return;
        }

        // After long inactivity, skip synchronous fallback probe briefly
        // so UI can recover first.
        if self.minimized_duration_secs >= 10.0
            && self.last_restore_time.elapsed() < Duration::from_secs(8)
        {
            return;
        }

        // For non-USN filesystems, do not block consistency probes based on a sticky
        // peak metric: reliability is more important and probe interval is already long.
        // Keep the guard for USN paths, where known-bad drives can probe as fast as 3s.
        let is_non_usn_fs = self
            .watcher_fallback_fs
            .as_deref()
            .map(|fs| !(fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS")))
            .unwrap_or(false);
        if !is_non_usn_fs && self.frame_time_peak_ms > 25.0 {
            return;
        }

        let interval = self.fallback_poll_interval(self.all_items.len());
        if self.watcher_fallback_last_probe.elapsed() < interval {
            return;
        }
        self.watcher_fallback_last_probe = Instant::now();

        let current_path = PathBuf::from(&self.navigation_state.current_path);
        let ui_signature = Self::compute_entries_signature(&self.all_items);
        self.watcher_fallback_signature = Some(ui_signature);

        let is_onedrive = crate::infrastructure::onedrive::is_onedrive_path(&current_path);
        let disk_entries =
            match crate::infrastructure::windows::hdd_directory_reader::read_directory_hdd_optimized(
                current_path.as_path(),
                is_onedrive,
            ) {
                Ok(entries) => entries,
                Err(err) => {
                    // If the directory doesn't exist anymore, navigate up.
                    if !current_path.is_dir() {
                        log::warn!(
                            "[FS-WATCH-FALLBACK] Current folder vanished: {:?} - navigating up",
                            current_path
                        );
                        self.navigate_to_nearest_valid_ancestor();
                        return;
                    }
                    log::debug!(
                        "[FS-WATCH-FALLBACK] Poll read failed for {:?}: {}",
                        current_path,
                        err
                    );
                    return;
                }
            };

        let disk_signature = Self::compute_entries_signature(&disk_entries);
        if disk_signature == ui_signature {
            return;
        }

        // Drift detected! RDCW missed cross-process events on this drive.
        // Record the verdict so future visits skip verification mode.
        let drive_letter =
            crate::infrastructure::windows::extract_drive_letter(current_path.as_path());
        if let Some(dl) = drive_letter {
            if !self.rdcw_unreliable_drives.get(&dl).copied().unwrap_or(false) {
                log::warn!(
                    "[FS-WATCH-FALLBACK] RDCW verified UNRELIABLE for drive {}:\\ (fs={:?}). Escalating to active polling.",
                    dl,
                    self.watcher_fallback_fs
                );
                self.rdcw_unreliable_drives.insert(dl, true);
            }
        }

        log::warn!(
            "[FS-WATCH-FALLBACK] Listing drift detected on {:?} (fs={:?}); scheduling reload",
            current_path,
            self.watcher_fallback_fs
        );

        self.directory_cache.invalidate(&current_path);
        if let Some(di) = &self.directory_index {
            let _ = di.invalidate(&current_path);
        }
        pending_disk_cache_invalidations.push(current_path.clone());
        self.watcher_fallback_signature = Some(disk_signature);
        self.pending_auto_reload = true;
    }

    pub(super) fn process_watcher_events_and_auto_reload(
        &mut self,
        current_path_norm: &str,
    ) -> WatcherPerfMarks {
        let internal_cache_root_norm =
            dirs::data_local_dir().map(|d| Self::normalize_for_match(&d.join("MTT-File-Manager")));
        let internal_cache_root_prefix = internal_cache_root_norm
            .as_ref()
            .map(|root| format!("{root}\\"));

        let watcher_start = Instant::now();
        self.drive_watcher.check_pending_activation();

        let recently_restored = self.minimized_duration_secs >= 10.0
            && self.last_restore_time.elapsed() < Duration::from_secs(10);
        let (max_batches, max_events) = if self.layout.saved_is_minimized {
            (1usize, 32usize)
        } else if recently_restored {
            (2usize, 96usize)
        } else if self.frame_time_peak_ms > 33.33 {
            (2usize, 128usize)
        } else if self.frame_time_peak_ms > 25.0 {
            (3usize, 192usize)
        } else {
            (4usize, 320usize)
        };

        let (drive_events, dropped_drive_events) = self
            .drive_watcher
            .poll_events_limited(max_batches, max_events);
        let t_poll_done = Instant::now();
        let drive_event_count = drive_events.len();

        if dropped_drive_events > 0 && !self.layout.saved_is_minimized {
            if dropped_drive_events >= max_events.saturating_mul(4) {
                log::warn!(
                    "[FS-WATCH] Dropped {} queued drive events after inactivity burst (kept={} batches<= {}, events<= {}). Scheduling safety reload.",
                    dropped_drive_events,
                    drive_event_count,
                    max_batches,
                    max_events
                );
            } else {
                log::debug!(
                    "[FS-WATCH] Dropped {} queued drive events (kept={} batches<= {}, events<= {})",
                    dropped_drive_events,
                    drive_event_count,
                    max_batches,
                    max_events
                );
            }
            if !self.navigation_state.is_computer_view && !self.navigation_state.is_recycle_bin_view
            {
                self.directory_cache
                    .invalidate(&PathBuf::from(&self.navigation_state.current_path));
                self.pending_auto_reload = true;
            }
        }

        #[cfg(feature = "notify-watcher")]
        let drive_watcher_active = !drive_events.is_empty();

        for event in &drive_events {
            if let crate::infrastructure::drive_watcher::DriveWatcherEvent::DriveLost(drive_root) =
                event
            {
                log::warn!("[FS-WATCH] DriveLost signal received for: {:?}", drive_root);
                self.drive_state.last_drive_refresh = Instant::now();
                self.reload_drive_list_async();

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

        const MAX_EVENTS_INDIVIDUAL: usize = 50;
        const FLOOD_RELOAD_COOLDOWN_MS: u64 = 5000;

        let mut pending_disk_cache_invalidations: Vec<PathBuf> = Vec::new();
        let mut folders_with_changed_contents: HashSet<PathBuf> = HashSet::new();

        self.process_drive_events_batch(
            &drive_events,
            current_path_norm,
            internal_cache_root_norm.as_deref(),
            internal_cache_root_prefix.as_deref(),
            MAX_EVENTS_INDIVIDUAL,
            FLOOD_RELOAD_COOLDOWN_MS,
            &mut pending_disk_cache_invalidations,
            &mut folders_with_changed_contents,
        );

        self.apply_folder_content_change_invalidations(
            folders_with_changed_contents,
            &mut pending_disk_cache_invalidations,
        );

        let drive_events_done = Instant::now();
        let drive_poll_ms = t_poll_done.duration_since(watcher_start).as_millis();
        let drive_process_ms = drive_events_done.duration_since(t_poll_done).as_millis();
        if drive_events_done.duration_since(watcher_start).as_millis() > 50 {
            log::warn!(
                "[PERF-MSG] DriveWatcher: poll={}ms process={}ms events={} dropped={}",
                drive_poll_ms,
                drive_process_ms,
                drive_event_count,
                dropped_drive_events
            );
        }

        #[cfg(feature = "notify-watcher")]
        self.process_legacy_notify_events(
            drive_watcher_active,
            current_path_norm,
            internal_cache_root_norm.as_deref(),
            internal_cache_root_prefix.as_deref(),
            MAX_EVENTS_INDIVIDUAL,
            &mut pending_disk_cache_invalidations,
        );

        self.maybe_poll_non_usn_consistency(&mut pending_disk_cache_invalidations);

        self.enqueue_disk_cache_invalidations(pending_disk_cache_invalidations);
        self.apply_watcher_reload_policy();

        let auto_reload_done = Instant::now();
        WatcherPerfMarks {
            watcher_start,
            drive_events_done,
            auto_reload_done,
        }
    }
}
