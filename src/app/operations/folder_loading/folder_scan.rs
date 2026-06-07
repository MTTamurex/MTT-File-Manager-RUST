use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::{is_image_extension, is_video_extension};
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_SYNC_FOLDER_SCAN_BATCH_BASE: usize = 24;
const MAX_SYNC_FOLDER_SCAN_BATCH_MODERATE: usize = 16;
const MAX_SYNC_FOLDER_SCAN_BATCH_CRITICAL: usize = 8;
const MAX_SYNC_FOLDER_SCAN_BATCH_NON_USN: usize = 18;
const MAX_SYNC_FOLDER_SCAN_BATCH_HARD_MAX: usize = 32;
const FOLDER_SCAN_RESOLVE_BUDGET_BASE_MS: u64 = 10;
const FOLDER_SCAN_RESOLVE_BUDGET_MODERATE_MS: u64 = 6;
const FOLDER_SCAN_RESOLVE_BUDGET_NON_USN_MS: u64 = 6;
const FOLDER_SCAN_RESOLVE_BUDGET_CRITICAL_MS: u64 = 3;
const FOLDER_SCAN_RESOLVE_BUDGET_HARD_MAX_MS: u64 = 14;

impl ImageViewerApp {
    /// Requests an async scan of a folder to discover the first image.
    /// OPTIMIZED: Sends message to a single worker (zero thread overhead)
    pub fn request_folder_scan(&mut self, folder_path: PathBuf) {
        self.request_folder_scans_batch(vec![folder_path]);
    }

    /// Batch version of request_folder_scan: resolves covers for multiple folders
    /// in a single SQLite query and calls filter_items() only once at the end.
    pub fn request_folder_scans_batch(&mut self, folder_paths: Vec<PathBuf>) {
        if folder_paths.is_empty() {
            return;
        }

        // Dedupe paths first: same folder can be requested multiple times in one frame.
        let mut seen = HashSet::with_capacity(folder_paths.len());
        let mut unique_paths = Vec::with_capacity(folder_paths.len());
        for path in folder_paths {
            if seen.insert(path.clone()) {
                unique_paths.push(path);
            }
        }

        // Protect UI thread: keep sync resolution bounded and push overflow to worker.
        let non_usn_fallback = self.watcher_fallback_polling
            && self
                .watcher_fallback_fs
                .as_deref()
                .map(|fs| !(fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS")))
                .unwrap_or(false);

        // Use live pressure (last/avg) instead of sticky peak to avoid over-throttling
        // thumbnail discovery for too long after a single transient spike.
        let frame_pressure_ms = self.last_actual_frame_ms.max(self.frame_time_avg_ms);
        let critical_pressure = frame_pressure_ms > 50.0;
        let moderate_pressure = !critical_pressure && frame_pressure_ms > 33.33;

        // Adaptive behavior by open tab count:
        // - 1 tab: prioritize responsiveness (less deferral)
        // - 6+ tabs: keep stricter bounds to protect the UI thread
        let open_tabs = self.tab_manager.count().max(1);
        let (tab_sync_adjust, tab_budget_adjust_ms): (isize, i64) = if open_tabs <= 1 {
            (8, 4)
        } else if open_tabs <= 3 {
            (4, 2)
        } else if open_tabs >= 6 {
            (-4, -2)
        } else {
            (0, 0)
        };

        let base_sync_cap = if critical_pressure {
            MAX_SYNC_FOLDER_SCAN_BATCH_CRITICAL
        } else if moderate_pressure {
            MAX_SYNC_FOLDER_SCAN_BATCH_MODERATE
        } else if non_usn_fallback {
            MAX_SYNC_FOLDER_SCAN_BATCH_NON_USN
        } else {
            MAX_SYNC_FOLDER_SCAN_BATCH_BASE
        };

        let sync_cap_adjusted = if tab_sync_adjust >= 0 {
            base_sync_cap.saturating_add(tab_sync_adjust as usize)
        } else {
            base_sync_cap.saturating_sub((-tab_sync_adjust) as usize)
        };
        let sync_cap = sync_cap_adjusted.clamp(
            MAX_SYNC_FOLDER_SCAN_BATCH_CRITICAL,
            MAX_SYNC_FOLDER_SCAN_BATCH_HARD_MAX,
        );

        let base_resolve_budget_ms = if critical_pressure {
            FOLDER_SCAN_RESOLVE_BUDGET_CRITICAL_MS
        } else if moderate_pressure {
            FOLDER_SCAN_RESOLVE_BUDGET_MODERATE_MS
        } else if non_usn_fallback {
            FOLDER_SCAN_RESOLVE_BUDGET_NON_USN_MS
        } else {
            FOLDER_SCAN_RESOLVE_BUDGET_BASE_MS
        };
        let resolve_budget_ms = (base_resolve_budget_ms as i64 + tab_budget_adjust_ms).clamp(
            FOLDER_SCAN_RESOLVE_BUDGET_CRITICAL_MS as i64,
            FOLDER_SCAN_RESOLVE_BUDGET_HARD_MAX_MS as i64,
        ) as u64;
        let resolve_budget = Duration::from_millis(resolve_budget_ms);

        let mut overflow_paths = if unique_paths.len() > sync_cap {
            unique_paths.split_off(sync_cap)
        } else {
            Vec::new()
        };

        let deferred_to_worker = overflow_paths.len();
        if deferred_to_worker > 0 {
            for path in overflow_paths.drain(..) {
                let _ = self.cover_worker_sender.send(path);
            }
        }

        if unique_paths.is_empty() {
            return;
        }

        let requested_folders = unique_paths.len();
        let all_items_len_before = self.all_items.len();
        let items_len_before = self.items.len();
        let total_start = Instant::now();

        // 1. Single batched SQLite query for all folder covers
        let db_start = Instant::now();
        let db_covers = self.app_state_db.get_folder_covers(&unique_paths);
        let db_ms = db_start.elapsed().as_millis();

        // 2. Resolve covers: DB hit → DirectoryIndex fallback → worker fallback
        let mut resolved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut worker_fallbacks: Vec<PathBuf> = Vec::new();
        // Folders whose DB-cached cover is used for immediate display but needs
        // background re-validation (network/virtual drives where fast_path_exists
        // could block 100-150ms on the UI thread).
        let mut worker_revalidations: Vec<PathBuf> = Vec::new();
        let resolve_start = Instant::now();
        let mut resolve_budget_exhausted = false;
        let mut worker_fallback_due_to_budget = 0usize;

        // On local drives (not network/virtual), validate that DB-cached cover
        // files still exist on disk.  GetFileAttributesW is ~0.01ms on NTFS but
        // can take 100-150ms on Cryptomator/VeraCrypt, so only enable when safe.
        let can_validate_cover_exists = unique_paths
            .first()
            .map(|p| !crate::infrastructure::io_priority::is_network_or_virtual(p))
            .unwrap_or(false);

        let mut unique_paths_iter = unique_paths.into_iter();
        while let Some(folder_path) = unique_paths_iter.next() {
            if resolve_start.elapsed() >= resolve_budget {
                resolve_budget_exhausted = true;
                let fallback_before = worker_fallbacks.len();
                worker_fallbacks.push(folder_path);
                worker_fallbacks.extend(unique_paths_iter);
                worker_fallback_due_to_budget =
                    worker_fallbacks.len().saturating_sub(fallback_before);
                break;
            }

            let cover_opt = if let Some(cover) = db_covers.get(&folder_path) {
                if is_invalid_cached_cover_path(cover) {
                    self.app_state_db.remove_folder_cover(&folder_path);
                    None
                } else if can_validate_cover_exists
                    && !crate::infrastructure::onedrive::fast_path_exists(cover)
                {
                    // Cover file was deleted externally — evict stale entry
                    // and let the cover worker re-discover.
                    self.app_state_db.remove_folder_cover(&folder_path);
                    self.cache_manager.texture_cache.pop(cover);
                    self.cache_manager.loading_set.remove(cover);
                    self.cache_manager.invalidate_folder_preview(&folder_path);
                    None
                } else {
                    // On network/virtual drives, queue for background re-validation
                    // so stale covers get corrected without blocking the UI thread.
                    if !can_validate_cover_exists {
                        worker_revalidations.push(folder_path.clone());
                    }
                    Some(cover.clone())
                }
            } else {
                // INDEX PATH: If DB has no cover, try DirectoryIndex (no HDD hit).
                // Use try_get_directory (non-blocking) to avoid stalling the UI thread
                // when a background worker holds the DirectoryIndex Mutex during put_directory.
                let mut found = None;
                if let Some(di) = &self.directory_index {
                    if let Some((_meta, files)) = di.try_get_directory(&folder_path) {
                        for file in files.iter() {
                            if file.is_dir {
                                continue;
                            }
                            if let Some(ext) = std::path::Path::new(&file.name)
                                .extension()
                                .and_then(|e| e.to_str())
                            {
                                if is_image_extension(ext) || is_video_extension(ext) {
                                    found = Some(folder_path.join(&file.name));
                                    break;
                                }
                            }
                        }
                    }
                }
                found
            };

            if let Some(cover) = cover_opt {
                resolved.push((folder_path, cover));
            } else {
                worker_fallbacks.push(folder_path);
            }
        }
        let resolve_ms = resolve_start.elapsed().as_millis();
        let resolved_count = resolved.len();
        let worker_fallback_count = worker_fallbacks.len();
        let worker_fallback_missing_count =
            worker_fallback_count.saturating_sub(worker_fallback_due_to_budget);

        // 3. Apply resolved covers to items (single pass through all_items)
        let apply_start = Instant::now();
        let mut any_updated = false;
        let mut cover_thumbnail_probes: Vec<PathBuf> = Vec::new();
        if !resolved.is_empty() {
            // Build a lookup map for O(1) access
            let resolve_map: std::collections::HashMap<&PathBuf, &PathBuf> =
                resolved.iter().map(|(fp, cp)| (fp, cp)).collect();

            for item in self.all_items_mut().iter_mut() {
                if let Some(cover) = resolve_map.get(&item.path) {
                    if item.folder_cover.as_ref() != Some(cover) {
                        item.folder_cover = Some((*cover).clone());
                        any_updated = true;
                    }
                    cover_thumbnail_probes.push((*cover).clone());
                }
            }

            // Persist covers to DB — non-blocking to avoid stalling the
            // UI thread when a worker holds the SQLite writer lock.
            // Covers that fail to persist will be re-discovered on next visit
            // or saved by the background cover worker.
            for (folder_path, cover) in &resolved {
                self.app_state_db.try_set_folder_cover(folder_path, cover);
            }
        }
        let apply_ms = apply_start.elapsed().as_millis();

        // Probe cover thumbnails to keep the worker retry/defer path alive for
        // unsafe media and to warm SQLite for folder preview composition. The UI
        // upload pipeline discards these cover-only results before `load_texture`,
        // so this no longer creates a second visible GPU upload wave.
        let thumbs_start = Instant::now();
        let mut thumb_requests = 0usize;
        for cover in cover_thumbnail_probes {
            if !self.cache_manager.has_thumbnail(&cover)
                && self.cache_manager.start_loading(cover.clone())
            {
                self.request_thumbnail_load(cover, 256);
                thumb_requests += 1;
            }
        }
        let thumbs_ms = thumbs_start.elapsed().as_millis();

        // 4. Single filter_items() call at the end
        let filter_start = Instant::now();
        let mut filter_ms = 0u128;
        if any_updated && !(self.hold_visible_items_until_load_complete && self.is_loading_folder) {
            self.filter_items();
            self.ui_ctx.request_repaint();
            filter_ms = filter_start.elapsed().as_millis();
        }

        // 5. Send remaining folders to worker for HDD scan
        for path in worker_fallbacks {
            let _ = self.cover_worker_sender.send(path);
        }

        // 6. Queue background re-validation for network/virtual drive covers
        //    that were shown from DB cache without existence check.
        for path in worker_revalidations {
            let _ = self.cover_worker_sender.send(path);
        }

        let total_ms = total_start.elapsed().as_millis();
        let should_warn = total_ms > 40;
        let should_debug_budget_only = !should_warn && resolve_budget_exhausted;

        if should_warn {
            log::warn!(
                "[PERF-FOLDER-SCAN] total={}ms db={}ms resolve={}ms apply={}ms thumbs={}ms filter={}ms | requested={} resolved={} worker_fallback={} worker_fallback_miss={} worker_fallback_budget={} deferred_worker={} sync_cap={} resolve_budget_ms={} resolve_budget_exit={} non_usn_fallback={} thumb_requests={} all_items={} items={}",
                total_ms,
                db_ms,
                resolve_ms,
                apply_ms,
                thumbs_ms,
                filter_ms,
                requested_folders,
                resolved_count,
                worker_fallback_count,
                worker_fallback_missing_count,
                worker_fallback_due_to_budget,
                deferred_to_worker,
                sync_cap,
                resolve_budget_ms,
                resolve_budget_exhausted,
                non_usn_fallback,
                thumb_requests,
                all_items_len_before,
                items_len_before,
            );
        } else if should_debug_budget_only {
            log::debug!(
                "[PERF-FOLDER-SCAN] total={}ms db={}ms resolve={}ms apply={}ms thumbs={}ms filter={}ms | requested={} resolved={} worker_fallback={} worker_fallback_miss={} worker_fallback_budget={} deferred_worker={} sync_cap={} resolve_budget_ms={} resolve_budget_exit={} non_usn_fallback={} thumb_requests={} all_items={} items={}",
                total_ms,
                db_ms,
                resolve_ms,
                apply_ms,
                thumbs_ms,
                filter_ms,
                requested_folders,
                resolved_count,
                worker_fallback_count,
                worker_fallback_missing_count,
                worker_fallback_due_to_budget,
                deferred_to_worker,
                sync_cap,
                resolve_budget_ms,
                resolve_budget_exhausted,
                non_usn_fallback,
                thumb_requests,
                all_items_len_before,
                items_len_before,
            );
        }

        if should_warn || should_debug_budget_only {
            log::debug!(
                "[PERF-FOLDER-SCAN-STATE] frame_pressure_ms={:.1} last_actual_ms={:.1} avg_frame_ms={:.1} critical_pressure={} moderate_pressure={} peak_ms={:.1} tabs={} base_sync_cap={} tab_sync_adjust={} base_resolve_budget_ms={} tab_budget_adjust_ms={}",
                frame_pressure_ms,
                self.last_actual_frame_ms,
                self.frame_time_avg_ms,
                critical_pressure,
                moderate_pressure,
                self.frame_time_peak_ms,
                open_tabs,
                base_sync_cap,
                tab_sync_adjust,
                base_resolve_budget_ms,
                tab_budget_adjust_ms,
            );
        }
    }
}

fn is_invalid_cached_cover_path(path: &Path) -> bool {
    // NOTE: Do NOT call fast_path_exists / GetFileAttributesW here.
    // On virtual/encrypted drives (Cryptomator, Z:) a single
    // GetFileAttributesW call can take 100-150ms, freezing the UI thread.
    // Stale covers are harmless: the thumbnail worker will fail to load the
    // missing file and the entry is cleaned up lazily.

    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ts"))
}
