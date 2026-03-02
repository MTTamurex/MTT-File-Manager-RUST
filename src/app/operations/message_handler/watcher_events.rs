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
                // Probe runs in background thread — safe to use shorter intervals.
                if item_count <= 500 {
                    Duration::from_secs(10)
                } else if item_count <= 2_000 {
                    Duration::from_secs(15)
                } else {
                    Duration::from_secs(25)
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
            // Unverified non-USN: probe in background, moderate interval.
            Duration::from_secs(20)
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

    /// Sends a consistency probe request to the background worker if the interval has elapsed.
    /// The actual disk read happens off the UI thread.
    fn maybe_send_consistency_probe(&mut self) {
        if !self.watcher_fallback_polling {
            return;
        }
        if self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            return;
        }
        if self.is_loading_folder
            || self.file_operation_state.file_ops_in_progress > 0
            || self.layout.saved_is_minimized
        {
            return;
        }

        // After long inactivity, skip probe briefly so UI can recover first.
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

        // Collect cover paths from subfolder entries for staleness verification.
        let cover_paths: Vec<(PathBuf, PathBuf)> = self
            .all_items
            .iter()
            .filter_map(|entry| {
                if entry.is_dir {
                    entry.folder_cover.as_ref().map(|cover| {
                        (current_path.join(&entry.name), cover.clone())
                    })
                } else {
                    None
                }
            })
            .collect();

        let is_onedrive = crate::infrastructure::onedrive::is_onedrive_path(&current_path);
        let _ = self.consistency_probe_tx.send(
            crate::app::init_workers::consistency_probe_worker::ConsistencyProbeRequest {
                path: current_path,
                is_onedrive,
                ui_signature,
                cover_paths,
            },
        );
    }

    /// Processes results from the async consistency probe worker.
    /// Handles drift detection, cache invalidation, stale covers, and folder-vanished scenarios.
    fn process_consistency_probe_results(
        &mut self,
        pending_disk_cache_invalidations: &mut Vec<PathBuf>,
    ) {
        while let Ok(result) = self.consistency_probe_rx.try_recv() {
            let current_path = PathBuf::from(&self.navigation_state.current_path);

            // Discard stale results from a different folder.
            if result.path != current_path {
                continue;
            }

            if result.path_vanished {
                log::warn!(
                    "[FS-WATCH-FALLBACK] Current folder vanished: {:?} - navigating up",
                    result.path
                );
                self.navigate_to_nearest_valid_ancestor();
                return;
            }

            // Handle stale subfolder covers: clear in-memory cover and re-request discovery.
            if !result.stale_covers.is_empty() {
                log::debug!(
                    "[FS-WATCH-FALLBACK] {} stale cover(s) detected in {:?}",
                    result.stale_covers.len(),
                    result.path.file_name().unwrap_or_default()
                );
                let stale_set: HashSet<PathBuf> = result.stale_covers.into_iter().collect();
                for item in &mut self.all_items {
                    if !item.is_dir || item.folder_cover.is_none() {
                        continue;
                    }
                    let folder_path = current_path.join(&item.name);
                    if stale_set.contains(&folder_path) {
                        item.folder_cover = None;
                        // Evict stale composed preview so it's re-composed with the
                        // new cover once the cover_worker returns.
                        self.cache_manager.invalidate_folder_preview(&folder_path);
                        self.scanned_folders.pop(&folder_path);
                        // Invalidate disk cache cover in background (avoids Mutex on UI thread).
                        pending_disk_cache_invalidations.push(folder_path.clone());
                        // Re-request cover discovery for this subfolder.
                        let _ = self.cover_worker_sender.send(folder_path);
                    }
                }
                self.pending_items_rebuild = true;
            }

            // Re-check UI signature in case items changed while probe was in flight.
            let current_ui_signature = Self::compute_entries_signature(&self.all_items);
            if result.disk_signature == current_ui_signature {
                continue;
            }

            // Drift detected! RDCW missed cross-process events on this drive.
            let drive_letter =
                crate::infrastructure::windows::extract_drive_letter(result.path.as_path());
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
                result.path,
                self.watcher_fallback_fs
            );

            self.directory_cache.invalidate(&result.path);
            if let Some(di) = &self.directory_index {
                let _ = di.invalidate(&result.path);
            }
            pending_disk_cache_invalidations.push(result.path);
            self.watcher_fallback_signature = Some(result.disk_signature);
            self.pending_auto_reload = true;
        }
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

        // While a shell file operation is in progress (copy/move/delete), drain
        // watcher events without processing them individually.  This avoids
        // synchronous filesystem syscalls (is_dir, GetFileAttributesW) on the UI
        // thread while the disk is under heavy I/O.  A full folder reload is
        // triggered in handle_file_operation_finished() once all ops complete.
        if self.file_operation_state.file_ops_in_progress > 0 {
            let (_drained, _dropped) = self
                .drive_watcher
                .poll_events_limited(max_batches, max_events);
            if !self.navigation_state.is_computer_view
                && !self.navigation_state.is_recycle_bin_view
            {
                self.pending_auto_reload = true;
            }
            let now = Instant::now();
            return WatcherPerfMarks {
                watcher_start,
                drive_events_done: now,
                auto_reload_done: now,
            };
        }

        let (drive_events, dropped_drive_events) = self
            .drive_watcher
            .poll_events_limited(max_batches, max_events);
        let t_poll_done = Instant::now();
        let drive_event_count = drive_events.len();

        if dropped_drive_events > 0 && !self.layout.saved_is_minimized {
            if dropped_drive_events >= max_events.saturating_mul(4) {
                log::warn!(
                    "[FS-WATCH] Dropped {} queued drive events (event burst overflow, kept={} batches<= {}, events<= {}). Scheduling safety reload.",
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

        // Async consistency probe: receive results from background worker
        self.process_consistency_probe_results(&mut pending_disk_cache_invalidations);
        // Send new probe request if interval elapsed (disk read happens in background)
        self.maybe_send_consistency_probe();

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
