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
                // Non-USN known-bad: aggressive polling since RDCW is confirmed broken.
                if item_count <= 500 {
                    Duration::from_secs(2)
                } else if item_count <= 2_000 {
                    Duration::from_secs(3)
                } else {
                    Duration::from_secs(5)
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
            // Unverified non-USN: keep interval low so first drift is caught fast.
            Duration::from_secs(3)
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

        // Collect visible subfolder cover state so the non-USN consistency probe can
        // detect cover changes even when the directory listing itself is unchanged.
        let folder_cover_states: Vec<(PathBuf, Option<PathBuf>)> = self
            .all_items
            .iter()
            .filter_map(|entry| {
                if entry.is_dir {
                    Some((entry.path.clone(), entry.folder_cover.clone()))
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
                show_hidden_files: self.show_hidden_files,
                folder_cover_states,
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

            // Handle visible subfolder cover changes detected by the non-USN probe.
            if !result.changed_folder_covers.is_empty() {
                log::debug!(
                    "[FS-WATCH-FALLBACK] {} folder cover change(s) detected in {:?}",
                    result.changed_folder_covers.len(),
                    result.path.file_name().unwrap_or_default()
                );
                let changed_set: HashSet<PathBuf> =
                    result.changed_folder_covers.into_iter().collect();
                for folder_path in &changed_set {
                    // On non-NTFS fallback paths, cover changes can be the only
                    // signal that a subfolder's contents changed. Invalidate the
                    // folder-size caches alongside the cover so list rows and the
                    // details panel both re-read the updated size.
                    self.invalidate_folder_size_cache(folder_path);
                    self.invalidate_folder_cover_state(folder_path);
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

            // The consistency probe only tells us that the visible listing on
            // disk no longer matches the UI snapshot. Before reloading, evict
            // folder-size caches for currently visible directories so stale
            // totals are not reused after the fresh listing arrives.
            let visible_folder_paths: Vec<PathBuf> = self
                .all_items
                .iter()
                .filter(|item| item.is_dir)
                .map(|item| item.path.clone())
                .collect();
            for folder_path in visible_folder_paths {
                self.invalidate_folder_size_cache(&folder_path);
            }

            self.directory_dirty_registry.mark_dirty(&result.path);
            self.directory_cache.invalidate(&result.path);
            if let Some(di) = &self.directory_index {
                let _ = di.invalidate(&result.path);
            }
            pending_disk_cache_invalidations.push(result.path);
            self.watcher_fallback_signature = Some(result.disk_signature);

            // Consistency probe already confirmed disk != UI.
            // Reload IMMEDIATELY — skip the debounce timer so the user
            // sees the updated listing within one frame instead of
            // waiting for the next scheduled repaint.
            if !self.is_loading_folder
                && self.file_operation_state.file_ops_in_progress == 0
                && !self.navigation_state.is_computer_view
                && !self.navigation_state.is_recycle_bin_view
            {
                log::info!(
                    "[FS-WATCH] IMMEDIATE RELOAD from consistency probe for {:?}",
                    self.navigation_state.current_path
                );
                self.loaded_path.clear();
                self.load_folder(false);
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            } else {
                self.request_watcher_auto_reload();
            }
            self.ui_ctx.request_repaint();
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

        const MAX_EVENTS_INDIVIDUAL: usize = 50;

        let mut pending_disk_cache_invalidations: Vec<PathBuf> = Vec::new();

        let drive_events_done = Instant::now();

        #[cfg(feature = "notify-watcher")]
        self.process_legacy_notify_events(
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

        // Process deferred folder mtime rechecks (Windows lazy-write delay)
        self.process_pending_folder_mtime_rechecks();

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
