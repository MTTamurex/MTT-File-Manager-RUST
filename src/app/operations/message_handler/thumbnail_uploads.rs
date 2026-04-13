use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_PENDING_THUMBNAILS: usize = 64;
const MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME: usize = 96;
const CRITICAL_FRAME_TIME_MS: f32 = 33.33;
const SEVERE_FRAME_TIME_MS: f32 = 25.0;
const MAX_INCOMING_THUMBNAIL_BUDGET_MS: u64 = 4;
const MIN_INCOMING_THUMBNAIL_BUDGET_MS: u64 = 2;
const TEXTURE_CACHE_RETUNE_INTERVAL_MS: u64 = 900;
const TEXTURE_CACHE_RETUNE_MIN_DELTA_ITEMS: usize = 16;

fn live_frame_pressure_ms(app: &ImageViewerApp) -> f32 {
    app.last_actual_frame_ms.max(app.frame_time_avg_ms)
}

fn compute_texture_cache_target_items(
    app: &ImageViewerApp,
    frame_pressure_ms: f32,
    open_tabs: usize,
    is_scrolling: bool,
    is_video_playing: bool,
) -> usize {
    let tab_factor: f32 = if open_tabs <= 1 {
        1.25
    } else if open_tabs <= 3 {
        1.12
    } else if open_tabs <= 5 {
        1.0
    } else {
        0.90
    };

    let queue_pending = app.thumbnail_queue.pending_count();
    let upload_pending = app.pending_thumbnails.len();
    let pipeline_backlog = queue_pending.saturating_add(upload_pending);

    let backlog_boost = if pipeline_backlog >= 900 {
        96
    } else if pipeline_backlog >= 600 {
        72
    } else if pipeline_backlog >= 320 {
        48
    } else if pipeline_backlog >= 140 {
        24
    } else {
        0
    };

    let frame_headroom_boost = if frame_pressure_ms < 12.0 {
        24
    } else if frame_pressure_ms < 16.0 {
        12
    } else {
        0
    };

    let frame_penalty = if frame_pressure_ms > CRITICAL_FRAME_TIME_MS {
        80
    } else if frame_pressure_ms > SEVERE_FRAME_TIME_MS {
        52
    } else if frame_pressure_ms > 20.0 {
        24
    } else {
        0
    };

    let activity_penalty = if is_video_playing && is_scrolling {
        48
    } else if is_video_playing {
        30
    } else if is_scrolling {
        12
    } else {
        0
    };

    let raw_target = ((220.0_f32 * tab_factor).round() as i32)
        + backlog_boost
        + frame_headroom_boost
        - frame_penalty
        - activity_penalty;

    raw_target.clamp(140, 420) as usize
}

impl ImageViewerApp {
    pub(super) fn process_thumbnail_upload_pipeline(&mut self, ctx: &egui::Context) -> bool {
        let mut received_any = false;
        let mut incoming_count = 0usize;
        let mut has_more_incoming = false;
        let is_burst = self.is_in_restore_burst();
        let frame_pressure_ms = live_frame_pressure_ms(self);
        // During burst, ignore frame pressure for intake — the slow frames are caused
        // by OS paging, not by actual rendering load.  A generous budget lets us
        // drain the worker channel and queue items for upload faster.
        let incoming_budget = if is_burst {
            Duration::from_millis(8)
        } else if frame_pressure_ms > CRITICAL_FRAME_TIME_MS {
            Duration::from_millis(MIN_INCOMING_THUMBNAIL_BUDGET_MS)
        } else {
            Duration::from_millis(MAX_INCOMING_THUMBNAIL_BUDGET_MS)
        };
        let incoming_start = Instant::now();
        let mut not_found_failures: Vec<PathBuf> = Vec::new();
        let mut successful_thumb_paths: Vec<PathBuf> = Vec::new();
        let eviction_visible: Option<HashSet<PathBuf>> = self.visible_index_range
            .and_then(|(min_vis, max_vis)| {
                let items = &self.items;
                if items.is_empty() {
                    return None;
                }
                let max_vis = max_vis.min(items.len().saturating_sub(1));
                Some((min_vis..=max_vis)
                    .map(|i| items[i].path.clone())
                    .collect())
            });

        // Reduce intake when pending queue is already backlogged to spread
        // GPU upload work across more frames and prevent frame-time spikes.
        // During burst mode, skip the throttle — we want to fill the queue fast.
        let effective_incoming_cap = if is_burst {
            MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME
        } else if self.pending_thumbnails.len() > 32 {
            24
        } else {
            MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME
        };

        while incoming_count < effective_incoming_cap {
            if incoming_start.elapsed() >= incoming_budget {
                has_more_incoming = true;
                break;
            }
            let thumbnail_data = match self.image_receiver.try_recv() {
                Ok(data) => data,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
            };

            incoming_count += 1;
            if thumbnail_data.generation != self.generation {
                self.cache_manager.finish_loading(&thumbnail_data.path);
                self.cache_manager
                    .finish_pending_upload(&thumbnail_data.path);
                continue;
            }

            // Reject stale results that were in-flight when the path was evicted
            // (rename/delete via watcher).  The counter was incremented by
            // evict_stale_path_caches; decrement here and skip the data.
            if let Some(skip_count) = self.thumbnail_eviction_skips.get_mut(&thumbnail_data.path) {
                if *skip_count > 0 {
                    *skip_count -= 1;
                    if *skip_count == 0 {
                        self.thumbnail_eviction_skips.remove(&thumbnail_data.path);
                    }
                    self.cache_manager.finish_loading(&thumbnail_data.path);
                    log::debug!(
                        "[THUMB-UPLOAD] Rejected stale in-flight thumbnail for {:?} (eviction skip)",
                        thumbnail_data.path.file_name().unwrap_or_default()
                    );
                    continue;
                }
            }

            self.cache_manager.finish_loading(&thumbnail_data.path);

            if thumbnail_data.image_data.is_empty() {
                if thumbnail_data.not_found {
                    self.cache_manager
                        .mark_as_failed(thumbnail_data.path.clone());
                    not_found_failures.push(thumbnail_data.path.clone());
                } else {
                    log::debug!(
                        "[THUMB-UPLOAD] transient thumbnail miss for {:?}; allowing automatic retry",
                        thumbnail_data.path
                    );
                }

                continue;
            }

            while self.pending_thumbnails.len() >= if is_burst { MAX_PENDING_THUMBNAILS * 3 } else { MAX_PENDING_THUMBNAILS } {
                // FIX: Smart eviction — prefer removing off-screen items to keep visible
                // ones alive. On SSD, the worker queue processes most-recently-added items
                // first (LIFO), so items from the user's final scroll position arrive first
                // and sit at the front of the deque. Blind FIFO eviction (pop_front) would
                // eject exactly the items the user is looking at. Instead, scan for the
                // first off-screen item and evict it; fall back to FIFO only when every
                // pending item is visible (extremely unlikely with MAX_PENDING=64).
                let evict_idx = eviction_visible.as_ref()
                    .and_then(|visible| {
                        // Find first off-screen item in the deque
                        self.pending_thumbnails
                            .iter()
                            .position(|t| !visible.contains(&t.path))
                    });

                match evict_idx {
                    Some(idx) => {
                        if let Some(old) = self.pending_thumbnails.remove(idx) {
                            self.cache_manager.finish_pending_upload(&old.path);
                        }
                    }
                    None => {
                        // All items visible — fall back to FIFO
                        if let Some(old) = self.pending_thumbnails.pop_front() {
                            self.cache_manager.finish_pending_upload(&old.path);
                        }
                    }
                }
            }

            self.cache_manager
                .start_pending_upload(thumbnail_data.path.clone());
            successful_thumb_paths.push(thumbnail_data.path.clone());
            self.pending_thumbnails.push_back(thumbnail_data);
            received_any = true;
        }

        if incoming_count >= effective_incoming_cap {
            has_more_incoming = true;
        }

        if self.handle_missing_cover_sources(not_found_failures) {
            received_any = true;
        }

        // When a successfully-loaded thumbnail belongs to a file that is
        // currently set as a folder_cover AND the folder has no SQLite-cached
        // preview, the visible preview is a stale compose_empty placeholder
        // (our MediaUnsafe fix skips SQLite persistence for those).
        //
        // Re-queue the folder for composition WITHOUT invalidating the GPU
        // cache — the old placeholder stays visible until the worker produces
        // the new preview, avoiding any visible flicker.
        if !successful_thumb_paths.is_empty() {
            let successful_set: HashSet<&PathBuf> = successful_thumb_paths.iter().collect();
            let mut parent_folders_needing_scan = HashSet::new();

            for thumb_path in &successful_thumb_paths {
                let Some(parent) = thumb_path.parent() else {
                    continue;
                };

                if self.all_items.iter().any(|item| {
                    item.is_dir
                        && item.folder_cover.is_none()
                        && item.path == parent
                }) {
                    parent_folders_needing_scan.insert(parent.to_path_buf());
                }
            }

            if !parent_folders_needing_scan.is_empty() {
                self.request_folder_scans_batch(parent_folders_needing_scan.into_iter().collect());
            }

            for item in &self.all_items {
                if let Some(ref cover) = item.folder_cover {
                    if item.is_dir && successful_set.contains(cover) {
                        // SQLite miss ⇒ the current preview was a MediaUnsafe placeholder.
                        // SQLite hit  ⇒ preview already composed with real media — skip.
                        if self.disk_cache.get_folder_preview_cache(&item.path).is_none() {
                            if self.cache_manager.start_folder_preview_loading(item.path.clone()) {
                                let _ = self.folder_preview_sender.try_send(item.path.clone());
                            }
                            log::debug!(
                                "[FOLDER PREVIEW] Re-composing {:?} (cover {:?} now available)",
                                item.path.file_name().unwrap_or_default(),
                                cover.file_name().unwrap_or_default(),
                            );
                        }
                    }
                }
            }
        }

        // Hold the scrolling throttle slightly longer than a single wheel tick.
        // With smooth visual lerp enabled, the grid can still be moving after the
        // last input event; releasing heavy thumbnail work too early causes
        // intermittent frame drops in the middle of scroll sequences.
        let is_scrolling = self.last_scroll_time.elapsed() < Duration::from_millis(180);
        let is_video_playing = self.is_video_playing_docked();
        // During burst, suppress pressure flags so the upload loop doesn't defer
        // off-screen items or reduce visible-only mode.
        let is_performance_critical = !is_burst && frame_pressure_ms > CRITICAL_FRAME_TIME_MS;
        let is_performance_severe = !is_burst && frame_pressure_ms > SEVERE_FRAME_TIME_MS;

        let open_tabs = self.tab_manager.count().max(1);

        if self.last_texture_cache_retune.elapsed()
            >= Duration::from_millis(TEXTURE_CACHE_RETUNE_INTERVAL_MS)
        {
            let queue_pending = self.thumbnail_queue.pending_count();
            let upload_pending = self.pending_thumbnails.len();
            // During burst, report low pressure so the cache size stays at its
            // maximum — shrinking the LRU now would evict textures we just uploaded.
            let retune_pressure = if is_burst { 10.0 } else { frame_pressure_ms };
            let target_texture_items = compute_texture_cache_target_items(
                self,
                retune_pressure,
                open_tabs,
                is_scrolling,
                is_video_playing,
            );

            let current_texture_items = self.cache_manager.texture_cache.cap().get();
            if current_texture_items.abs_diff(target_texture_items)
                >= TEXTURE_CACHE_RETUNE_MIN_DELTA_ITEMS
            {
                let applied_texture_items = self
                    .cache_manager
                    .retune_texture_cache_capacity(target_texture_items);

                if applied_texture_items != current_texture_items {
                    log::debug!(
                        "[PERF-TEXTURE-CACHE] old={} new={} target={} frame_pressure_ms={:.1} tabs={} queue_pending={} upload_pending={} scrolling={} video={}",
                        current_texture_items,
                        applied_texture_items,
                        target_texture_items,
                        frame_pressure_ms,
                        open_tabs,
                        queue_pending,
                        upload_pending,
                        is_scrolling,
                        is_video_playing,
                    );
                }
            }
            self.last_texture_cache_retune = Instant::now();
        }

        let tab_upload_boost = if open_tabs <= 1 {
            1.25
        } else if open_tabs <= 3 {
            1.10
        } else if open_tabs >= 6 {
            0.90
        } else {
            1.0
        };

        // During restore burst, bypass the adaptive throttle entirely.
        // The slow frames are caused by OS page-faults on the RGBA RAM cache,
        // not by rendering complexity.  Restricting uploads only prolongs the
        // blank-tile period.  We allow up to 48 uploads/frame (clamped by the
        // generous time budget below) which fills the visible grid in ~2-3 seconds.
        let base_max_uploads = if is_burst {
            48
        } else if is_performance_critical {
            1
        } else if is_performance_severe {
            2
        } else if is_video_playing && is_scrolling {
            4
        } else if is_scrolling {
            6
        } else if is_video_playing {
            5
        } else {
            12
        };

        let base_max_uploads = ((base_max_uploads as f32) * tab_upload_boost)
            .round()
            .clamp(1.0, if is_burst { 64.0 } else { 20.0 }) as usize;

        let perf_scale = if is_burst {
            // During burst the frame_time_avg is inflated by OS paging; don't penalise.
            1.0
        } else if self.frame_time_avg_ms <= 0.0 {
            1.0
        } else if self.frame_time_avg_ms < 12.0 {
            1.25
        } else if self.frame_time_avg_ms < 18.0 {
            1.0
        } else if self.frame_time_avg_ms < 24.0 {
            0.85
        } else {
            0.7
        };
        let max_uploads_per_frame = if is_burst {
            // Burst: don't reduce through perf_scale; use the burst cap directly.
            base_max_uploads
        } else {
            ((base_max_uploads as f32) * perf_scale)
                .round()
                .clamp(1.0, 20.0) as usize
        };

        let mut uploads_this_frame = 0;
        let upload_start = Instant::now();
        let now = Instant::now();
        if now.duration_since(self.last_upload_budget_update) > Duration::from_millis(750) {
            let target_budget_ms = if self.frame_time_avg_ms <= 0.0 {
                self.upload_budget_ms
            } else if self.frame_time_avg_ms < 12.0 {
                8.0
            } else if self.frame_time_avg_ms < 18.0 {
                6.0
            } else if self.frame_time_avg_ms < 24.0 {
                4.0
            } else {
                3.0
            };
            if (self.upload_budget_ms - target_budget_ms).abs() >= 0.5 {
                self.upload_budget_ms = target_budget_ms.clamp(2.0, 10.0);
                let entries = [("upload_budget_ms", self.upload_budget_ms.to_string())];
                if !self.app_state_db.try_set_preferences_batch(&entries) {
                    log::debug!(
                        "[PERF-THUMB-UPLOAD] Skipped upload_budget_ms persist this frame (writer busy)"
                    );
                }
            }
            self.last_upload_budget_update = now;
        }

        let base_budget_ms = if is_burst {
            // Generous time budget during burst — worth spending frame time now
            // to avoid many more slow frames with blank tiles.
            16.0
        } else if is_video_playing && is_scrolling {
            self.upload_budget_ms * 0.6
        } else if is_video_playing {
            self.upload_budget_ms * 0.75
        } else if is_scrolling {
            self.upload_budget_ms * 0.85
        } else {
            self.upload_budget_ms
        };
        let upload_budget_ms = if is_burst {
            base_budget_ms
        } else {
            (base_budget_ms * perf_scale).clamp(2.0, 10.0)
        };
        let upload_budget = Duration::from_millis(upload_budget_ms.round() as u64);

        let mut prioritized_path: Option<PathBuf> = None;
        if let Some(selected_file) = &self.selected_file {
            prioritized_path = Some(selected_file.path.clone());
        }
        if let Some(path) = prioritized_path {
            if let Some(pos) = self
                .pending_thumbnails
                .iter()
                .position(|thumb| thumb.path == path)
            {
                if pos > 0 {
                    if let Some(selected_thumb) = self.pending_thumbnails.remove(pos) {
                        self.pending_thumbnails.push_front(selected_thumb);
                    }
                }
            }
        }

        let visible_paths: Option<&crate::ui::cache::FxHashSet<PathBuf>> = if is_scrolling {
            if self.visible_range_cached != self.visible_index_range {
                self.visible_paths_cache.clear();
                if let Some((min_idx, max_idx)) = self.visible_index_range {
                    let items = &self.items;
                    if !items.is_empty() {
                        let max_idx = max_idx.min(items.len().saturating_sub(1));
                        for i in min_idx..=max_idx {
                            self.visible_paths_cache.insert(items[i].path.clone());
                        }
                    }
                }
                self.visible_range_cached = self.visible_index_range;
            }

            if self.visible_paths_cache.is_empty() {
                None
            } else {
                Some(&self.visible_paths_cache)
            }
        } else {
            None
        };
        let mut deferred_count = 0;
        let offscreen_upload_budget = if is_scrolling {
            if is_performance_critical {
                0
            } else if is_performance_severe {
                1
            } else {
                2
            }
        } else {
            usize::MAX
        };
        let mut offscreen_uploads = 0usize;

        while uploads_this_frame < max_uploads_per_frame {
            if let Some(thumbnail_data) = self.pending_thumbnails.pop_front() {
                if upload_start.elapsed() >= upload_budget {
                    self.pending_thumbnails.push_front(thumbnail_data);
                    break;
                }

                if thumbnail_data.generation != self.generation {
                    self.cache_manager
                        .finish_pending_upload(&thumbnail_data.path);
                    continue;
                }

                if is_performance_critical {
                    if let Some(vis) = visible_paths {
                        if !vis.contains(&thumbnail_data.path) {
                            self.pending_thumbnails.push_back(thumbnail_data);
                            deferred_count += 1;
                            if deferred_count > max_uploads_per_frame * 2 {
                                break;
                            }
                            continue;
                        }
                    }
                }

                if let Some(vis) = visible_paths {
                    if !vis.contains(&thumbnail_data.path) {
                        if offscreen_uploads >= offscreen_upload_budget {
                            self.pending_thumbnails.push_back(thumbnail_data);
                            deferred_count += 1;
                            if deferred_count > max_uploads_per_frame * 3 {
                                break;
                            }
                            continue;
                        }

                        offscreen_uploads += 1;
                    }
                }

                let path = thumbnail_data.path;
                let width = thumbnail_data.width;
                let height = thumbnail_data.height;
                let rgba_data = thumbnail_data.image_data;

                // Skip upload for paths pending deletion — the item was removed
                // externally and the texture would be immediately stale.
                if self.file_operation_state.pending_deletions.contains_key(&path) {
                    self.cache_manager.finish_pending_upload(&path);
                    continue;
                }

                let is_selected = self
                    .selected_file
                    .as_ref()
                    .is_some_and(|selected_file| selected_file.path == path);

                let texture_name = path.to_string_lossy().into_owned();

                let texture = ctx.load_texture(
                    texture_name,
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                self.cache_manager.finish_pending_upload(&path);
                self.cache_manager
                    .put_rgba_data(path.clone(), rgba_data, width, height);
                self.cache_manager
                    .put_thumbnail(path, texture.clone());

                if is_selected {
                    self.selected_thumbnail = Some(texture);
                }

                uploads_this_frame += 1;
                received_any = true;
            } else {
                break;
            }
        }

        if !self.pending_thumbnails.is_empty() || has_more_incoming {
            ctx.request_repaint();
        }

        if received_any && (is_burst || incoming_count >= 32 || uploads_this_frame >= 8) {
            log::debug!(
                "[PERF-THUMB-UPLOAD] incoming={} uploads={} pending={} max_uploads={} upload_budget_ms={:.1} frame_pressure_ms={:.1} tabs={} burst={} critical={} severe={} scrolling={} video={}",
                incoming_count,
                uploads_this_frame,
                self.pending_thumbnails.len(),
                max_uploads_per_frame,
                upload_budget_ms,
                frame_pressure_ms,
                open_tabs,
                is_burst,
                is_performance_critical,
                is_performance_severe,
                is_scrolling,
                is_video_playing,
            );
        }

        self.process_folder_preview_uploads(ctx, is_performance_critical, is_video_playing);
        received_any
    }

    fn handle_missing_cover_sources(&mut self, missing_paths: Vec<PathBuf>) -> bool {
        if missing_paths.is_empty() {
            return false;
        }

        let failed_paths: HashSet<PathBuf> = missing_paths.into_iter().collect();
        if failed_paths.is_empty() {
            return false;
        }

        let mut folders_to_refresh: HashSet<PathBuf> = HashSet::new();
        let mut updated_any = false;
        let mut remaining_master = failed_paths.len();

        for item in self.all_items.iter_mut() {
            if remaining_master == 0 {
                break;
            }
            if item
                .folder_cover
                .as_ref()
                .is_some_and(|cover| failed_paths.contains(cover))
            {
                let folder_path = item.path.clone();
                item.folder_cover = None;
                self.app_state_db.remove_folder_cover(&folder_path);
                folders_to_refresh.insert(folder_path);
                updated_any = true;
                remaining_master = remaining_master.saturating_sub(1);
            }
        }

        let items = std::sync::Arc::make_mut(&mut self.items);
        let mut remaining_visible = failed_paths.len();
        for item in items.iter_mut() {
            if remaining_visible == 0 {
                break;
            }
            if item
                .folder_cover
                .as_ref()
                .is_some_and(|cover| failed_paths.contains(cover))
            {
                item.folder_cover = None;
                updated_any = true;
                remaining_visible = remaining_visible.saturating_sub(1);
            }
        }

        for folder_path in folders_to_refresh {
            let _ = self.cover_worker_sender.send(folder_path);
        }

        updated_any
    }

    fn process_folder_preview_uploads(
        &mut self,
        ctx: &egui::Context,
        is_performance_critical: bool,
        is_video_playing: bool,
    ) {
        let max_folder_uploads = if is_performance_critical {
            2
        } else if is_video_playing {
            6
        } else {
            20
        };

        // Time-budget folder preview uploads to avoid frame spikes.
        // Each ctx.load_texture() can take 5-15ms, so uncapped uploads
        // of 20 previews/frame could stall the UI for up to 300ms.
        let budget = Duration::from_millis(if is_performance_critical { 3 } else { 8 });
        let start = Instant::now();

        let mut folder_uploads = 0;
        while folder_uploads < max_folder_uploads {
            if folder_uploads > 0 && start.elapsed() >= budget {
                break;
            }
            if let Ok(data) = self.folder_preview_receiver.try_recv() {
                self.cache_manager.finish_folder_preview_loading(&data.path);

                if !data.rgba_data.is_empty() {
                    let mut texture_name = String::from("folder_preview_");
                    texture_name.push_str(data.path.to_string_lossy().as_ref());
                    let texture = ctx.load_texture(
                        texture_name,
                        egui::ColorImage::from_rgba_unmultiplied(
                            [data.width as usize, data.height as usize],
                            &data.rgba_data,
                        ),
                        egui::TextureOptions::LINEAR,
                    );

                    self.cache_manager.put_folder_preview(data.path, texture);
                }

                folder_uploads += 1;
            } else {
                break;
            }
        }

        if folder_uploads >= max_folder_uploads || (folder_uploads > 0 && start.elapsed() >= budget)
        {
            ctx.request_repaint();
        }
    }
}
