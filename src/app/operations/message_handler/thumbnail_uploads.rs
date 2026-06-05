use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, ViewMode};
use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::diagnostic_logger::{
    diag_info, field_bool, field_duration_ms, field_label, field_u64,
};
use crate::ui::cache::{
    MAX_DYNAMIC_TEXTURE_CACHE_ITEMS, MIN_DYNAMIC_TEXTURE_CACHE_ITEMS,
    VULKAN_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS,
};
use eframe::egui;
use rustc_hash::FxHashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME: usize = 96;
const CRITICAL_FRAME_TIME_MS: f32 = 33.33;
const SEVERE_FRAME_TIME_MS: f32 = 25.0;
const MAX_INCOMING_THUMBNAIL_BUDGET_MS: u64 = 4;
const MIN_INCOMING_THUMBNAIL_BUDGET_MS: u64 = 2;
const TEXTURE_CACHE_RETUNE_INTERVAL_MS: u64 = 900;
const TEXTURE_CACHE_RETUNE_MIN_DELTA_ITEMS: usize = 16;
const DIAG_SLOW_TEXTURE_UPLOAD_THRESHOLD: Duration = Duration::from_millis(8);
const VULKAN_MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME: usize = 48;

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
    let visible_base = app.current_dynamic_texture_keep_count() as f32;
    let tab_factor: f32 = if open_tabs <= 5 { 1.0 } else { 0.90 };

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

    let frame_headroom_boost = 0;

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

    let raw_target =
        ((visible_base * tab_factor).round() as i32) + backlog_boost + frame_headroom_boost
            - frame_penalty
            - activity_penalty;

    let max_texture_items = if app.is_vulkan_backend() {
        VULKAN_MAX_DYNAMIC_TEXTURE_CACHE_ITEMS
            .max(visible_base as usize)
            .min(MAX_DYNAMIC_TEXTURE_CACHE_ITEMS)
    } else {
        MAX_DYNAMIC_TEXTURE_CACHE_ITEMS
    };

    raw_target.clamp(
        MIN_DYNAMIC_TEXTURE_CACHE_ITEMS as i32,
        max_texture_items as i32,
    ) as usize
}

fn log_slow_texture_upload(
    kind: &'static str,
    elapsed: Duration,
    width: u32,
    height: u32,
    is_opengl: bool,
    is_vulkan: bool,
) {
    if elapsed < DIAG_SLOW_TEXTURE_UPLOAD_THRESHOLD {
        return;
    }

    diag_info(
        "thumbnail_upload",
        "slow_load_texture",
        &[
            field_label("kind", kind),
            field_duration_ms("elapsed", elapsed),
            field_u64("width", width as u64),
            field_u64("height", height as u64),
            field_bool("opengl", is_opengl),
            field_bool("vulkan", is_vulkan),
        ],
    );
}

fn insert_visible_upload_ranks(
    ranks: &mut FxHashMap<PathBuf, usize>,
    items: &[FileEntry],
    visible_index_range: Option<(usize, usize)>,
    base_rank: usize,
) -> usize {
    let Some((min_idx, max_idx)) = visible_index_range else {
        return 0;
    };
    if items.is_empty() {
        return 0;
    }

    let max_idx = max_idx.min(items.len().saturating_sub(1));
    if min_idx > max_idx {
        return 0;
    }

    let mut inserted = 0usize;
    for idx in min_idx..=max_idx {
        ranks
            .entry(items[idx].path.clone())
            .or_insert(base_rank + inserted);
        inserted += 1;
    }

    inserted
}

fn snapshot_items_for_upload_rank(
    snapshot: &crate::app::dual_panel::PanelSnapshot,
) -> &[FileEntry] {
    if snapshot.items_snapshot_compact && snapshot.items.is_empty() {
        snapshot.all_items.as_ref().as_slice()
    } else {
        snapshot.items.as_ref().as_slice()
    }
}

fn visible_thumbnail_upload_ranks(app: &ImageViewerApp) -> FxHashMap<PathBuf, usize> {
    let mut ranks = FxHashMap::default();
    let mut next_rank = 0usize;

    if matches!(app.view_mode, ViewMode::Grid | ViewMode::List) {
        next_rank += insert_visible_upload_ranks(
            &mut ranks,
            app.items.as_ref().as_slice(),
            app.visible_index_range,
            next_rank,
        );
    }

    if app.dual_panel_enabled {
        if let Some(snapshot) = app.dual_panel_inactive_state.as_ref() {
            if matches!(snapshot.view_mode, ViewMode::Grid | ViewMode::List) {
                insert_visible_upload_ranks(
                    &mut ranks,
                    snapshot_items_for_upload_rank(snapshot),
                    snapshot.visible_index_range,
                    next_rank,
                );
            }
        }
    }

    ranks
}

fn next_thumbnail_upload_index(
    pending_thumbnails: &VecDeque<ThumbnailData>,
    selected_path: Option<&PathBuf>,
    visible_ranks: &FxHashMap<PathBuf, usize>,
) -> Option<usize> {
    if let Some(selected_path) = selected_path {
        let selected_can_preempt =
            visible_ranks.is_empty() || visible_ranks.contains_key(selected_path);
        if let Some(pos) = pending_thumbnails
            .iter()
            .position(|thumb| &thumb.path == selected_path)
            .filter(|_| selected_can_preempt)
        {
            return Some(pos);
        }
    }

    pending_thumbnails
        .iter()
        .enumerate()
        .filter_map(|(idx, thumb)| visible_ranks.get(&thumb.path).map(|rank| (idx, *rank)))
        .min_by_key(|(_, rank)| *rank)
        .map(|(idx, _)| idx)
        .or_else(|| (!pending_thumbnails.is_empty()).then_some(0))
}

impl ImageViewerApp {
    pub(super) fn process_thumbnail_upload_pipeline(&mut self, ctx: &egui::Context) -> bool {
        let mut received_any = false;
        let mut incoming_count = 0usize;
        let mut has_more_incoming = false;
        let is_burst = self.is_in_restore_burst();
        let is_opengl = self.is_opengl_backend();
        let is_vulkan = self.is_vulkan_backend();
        let frame_pressure_ms = live_frame_pressure_ms(self);
        let is_scrolling = self.last_scroll_time.elapsed() < Duration::from_millis(180);
        // During burst, ignore frame pressure for intake — the slow frames are caused
        // by OS paging, not by actual rendering load.  A generous budget lets us
        // drain the worker channel and queue items for upload faster.
        let incoming_budget = if is_burst {
            if is_opengl {
                Duration::from_millis(4)
            } else {
                Duration::from_millis(8)
            }
        } else if frame_pressure_ms > CRITICAL_FRAME_TIME_MS {
            Duration::from_millis(MIN_INCOMING_THUMBNAIL_BUDGET_MS)
        } else {
            Duration::from_millis(MAX_INCOMING_THUMBNAIL_BUDGET_MS)
        };
        let incoming_start = Instant::now();
        let mut not_found_failures: Vec<PathBuf> = Vec::new();
        let mut successful_thumb_paths: Vec<PathBuf> = Vec::new();

        let mut eviction_visible = self.visible_grid_paths_snapshot();
        if let Some(detail_path) = self.detail_panel_folder_preview_path() {
            eviction_visible
                .get_or_insert_with(crate::ui::cache::FxHashSet::default)
                .insert(detail_path);
        }
        let visible_texture_keep = self.current_dynamic_texture_keep_count();
        if let Some(visible_paths) = eviction_visible.as_ref() {
            self.cache_manager.promote_visible(visible_paths);
        }
        if self.cache_manager.texture_cache.cap().get() < visible_texture_keep {
            self.cache_manager
                .retune_texture_cache_capacity(visible_texture_keep);
        }

        let visible_folder_preview_keep = self.current_dynamic_folder_preview_keep_count();
        if self.cache_manager.folder_preview_cache.cap().get() < visible_folder_preview_keep {
            self.cache_manager
                .retune_folder_preview_cache_capacity(visible_folder_preview_keep);
        }

        let visible_rgba_budget = self.current_thumbnail_rgba_budget_bytes();
        self.cache_manager.retune_rgba_budget(visible_rgba_budget);
        self.cache_manager
            .retune_rgba_cache_capacity(visible_texture_keep);

        let dynamic_pending_limit = self.current_pending_thumbnail_upload_limit();

        // Reduce intake when pending queue is already backlogged to spread
        // GPU upload work across more frames and prevent frame-time spikes.
        // During burst mode, skip the throttle — we want to fill the queue fast.
        let effective_incoming_cap = if is_opengl && is_scrolling {
            24
        } else if is_vulkan && is_scrolling {
            32
        } else if is_burst {
            if is_opengl {
                48
            } else if is_vulkan {
                VULKAN_MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME
            } else {
                MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME
            }
        } else if self.pending_thumbnails.len() > dynamic_pending_limit / 2 {
            if is_vulkan {
                24
            } else {
                48
            }
        } else if is_vulkan {
            VULKAN_MAX_INCOMING_THUMBNAIL_MSGS_PER_FRAME
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

            self.cache_manager
                .failed_thumbnails
                .pop(&thumbnail_data.path);
            crate::workers::thumbnail::clear_failure_cache(&thumbnail_data.path);

            if self.should_skip_folder_cover_thumbnail_upload(&thumbnail_data.path) {
                successful_thumb_paths.push(thumbnail_data.path.clone());
                received_any = true;
                continue;
            }

            if let Some(existing) = self
                .pending_thumbnails
                .iter_mut()
                .find(|pending| pending.path == thumbnail_data.path)
            {
                let existing_dim = existing.width.max(existing.height);
                let incoming_dim = thumbnail_data.width.max(thumbnail_data.height);
                if thumbnail_data.priority < existing.priority {
                    existing.priority = thumbnail_data.priority;
                }
                if incoming_dim > existing_dim {
                    *existing = thumbnail_data;
                }
                self.trim_pending_thumbnail_uploads_to_limit();
                continue;
            }

            let already_cached = self
                .cache_manager
                .texture_cache
                .peek(&thumbnail_data.path)
                .is_some_and(|texture| {
                    let size = texture.size();
                    let cached_dim = size[0].max(size[1]) as u32;
                    cached_dim >= thumbnail_data.width.max(thumbnail_data.height)
                });
            if already_cached {
                self.cache_manager
                    .thumbnail_trace
                    .record_upload_already_cached();
                continue;
            }

            let incoming_visible = eviction_visible
                .as_ref()
                .is_some_and(|visible| visible.contains(&thumbnail_data.path));
            let mut drop_incoming = false;
            while self.pending_thumbnails.len() >= dynamic_pending_limit {
                // Prefer removing off-screen items to keep visible ones alive.
                let evict_idx = eviction_visible.as_ref().and_then(|visible| {
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
                        if eviction_visible.is_some() {
                            if incoming_visible {
                                // All pending items are visible. Allow the visible upload
                                // queue to temporarily exceed the byte cap instead of
                                // dropping one visible tile and showing its fallback icon.
                                break;
                            } else {
                                drop_incoming = true;
                                break;
                            }
                        } else if let Some(old) = self.pending_thumbnails.pop_front() {
                            self.cache_manager.finish_pending_upload(&old.path);
                        } else {
                            break;
                        }
                    }
                }
            }

            if drop_incoming {
                self.cache_manager
                    .finish_pending_upload(&thumbnail_data.path);
                continue;
            }

            self.cache_manager
                .start_pending_upload(thumbnail_data.path.clone());
            successful_thumb_paths.push(thumbnail_data.path.clone());
            self.pending_thumbnails.push_back(thumbnail_data);
            self.trim_pending_thumbnail_uploads_to_limit();
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

                if self
                    .all_items
                    .iter()
                    .any(|item| item.is_dir && item.folder_cover.is_none() && item.path == parent)
                {
                    parent_folders_needing_scan.insert(parent.to_path_buf());
                }
            }

            if !parent_folders_needing_scan.is_empty() {
                self.request_folder_scans_batch(parent_folders_needing_scan.into_iter().collect());
            }

            for item in self.all_items.iter() {
                if let Some(ref cover) = item.folder_cover {
                    if item.is_dir && successful_set.contains(cover) {
                        if self.cache_manager.has_folder_preview(&item.path) {
                            continue;
                        }

                        // SQLite miss ⇒ the current preview was a MediaUnsafe placeholder.
                        // SQLite hit  ⇒ preview already composed with real media — skip.
                        if self
                            .disk_cache
                            .get_folder_preview_cache(
                                &item.path,
                                self.current_folder_preview_bucket_size(),
                            )
                            .is_none()
                        {
                            if self
                                .cache_manager
                                .start_folder_preview_loading(item.path.clone())
                            {
                                let request =
                                    crate::workers::folder_preview_worker::FolderPreviewRequest {
                                        path: item.path.clone(),
                                        size_px: self.effective_folder_preview_request_size_px(),
                                    };
                                if let Err(err) = self.folder_preview_sender.try_send(request) {
                                    let request = err.into_inner();
                                    self.cache_manager
                                        .finish_folder_preview_loading(&request.path);
                                }
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
        let is_video_playing = self.is_video_playing_docked();
        // During burst, suppress pressure flags so the upload loop doesn't defer
        // off-screen items or reduce visible-only mode.
        let is_performance_critical = !is_burst && frame_pressure_ms > CRITICAL_FRAME_TIME_MS;
        let is_performance_severe = !is_burst && frame_pressure_ms > SEVERE_FRAME_TIME_MS;

        let open_tabs = self.tab_manager.count().max(1);

        let freeze_cache_retune = is_opengl && is_burst;
        if !freeze_cache_retune
            && self.last_texture_cache_retune.elapsed()
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
            )
            .max(self.current_dynamic_texture_keep_count());

            let current_texture_items = self.cache_manager.texture_cache.cap().get();
            if current_texture_items.abs_diff(target_texture_items)
                >= TEXTURE_CACHE_RETUNE_MIN_DELTA_ITEMS
            {
                let retune_texture_items = if target_texture_items < current_texture_items {
                    let delta = current_texture_items - target_texture_items;
                    let shrink = if delta > 200 {
                        (delta / 3).max(64)
                    } else if delta > 64 {
                        (delta / 4).max(32)
                    } else {
                        16
                    };
                    current_texture_items
                        .saturating_sub(shrink)
                        .max(target_texture_items)
                } else {
                    target_texture_items
                };

                let applied_texture_items = self
                    .cache_manager
                    .retune_texture_cache_capacity(retune_texture_items);

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

            let target_folder_preview_items = self.current_dynamic_folder_preview_keep_count();
            let current_folder_preview_items = self.cache_manager.folder_preview_cache.cap().get();
            if current_folder_preview_items.abs_diff(target_folder_preview_items)
                >= TEXTURE_CACHE_RETUNE_MIN_DELTA_ITEMS
            {
                self.cache_manager
                    .retune_folder_preview_cache_capacity(target_folder_preview_items);
            }

            let target_rgba_budget = self.current_thumbnail_rgba_budget_bytes();
            self.cache_manager.retune_rgba_budget(target_rgba_budget);
            self.cache_manager
                .retune_rgba_cache_capacity(target_texture_items);
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
        //
        // On OpenGL (Glow / wgpu-GL) each `ctx.load_texture` call is synchronous
        // on the CPU thread.  Unlike DX12/Vulkan where wgpu queues the upload
        // asynchronously, OpenGL blocks until the driver finishes the transfer
        // (5-15 ms per thumbnail).  Apply more conservative per-frame caps to
        // prevent UI freezes and frame drops.
        let base_max_uploads = if is_burst {
            if is_opengl {
                if is_scrolling {
                    2
                } else {
                    4
                }
            } else if is_vulkan {
                if is_scrolling {
                    6
                } else {
                    16
                }
            } else {
                48
            }
        } else if is_performance_critical {
            1
        } else if is_performance_severe {
            if is_opengl {
                1
            } else {
                2
            }
        } else if is_video_playing && is_scrolling {
            if is_opengl {
                2
            } else if is_vulkan {
                3
            } else {
                4
            }
        } else if is_scrolling {
            if is_opengl {
                2
            } else if is_vulkan {
                3
            } else {
                6
            }
        } else if is_video_playing {
            if is_opengl {
                3
            } else if is_vulkan {
                4
            } else {
                5
            }
        } else {
            if is_opengl {
                8
            } else if is_vulkan {
                12
            } else {
                12
            }
        };

        let base_max_uploads = ((base_max_uploads as f32) * tab_upload_boost)
            .round()
            .clamp(
                1.0,
                if is_burst {
                    if is_opengl {
                        if is_scrolling {
                            2.0
                        } else {
                            12.0
                        }
                    } else {
                        if is_vulkan {
                            16.0
                        } else {
                            64.0
                        }
                    }
                } else {
                    if is_vulkan {
                        12.0
                    } else {
                        16.0
                    }
                },
            ) as usize;

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
            let max_uploads = if is_vulkan { 16.0 } else { 20.0 };
            ((base_max_uploads as f32) * perf_scale)
                .round()
                .clamp(1.0, max_uploads) as usize
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
            if is_opengl {
                if is_scrolling {
                    2.0
                } else {
                    5.0
                }
            } else if is_vulkan {
                if is_scrolling {
                    4.0
                } else {
                    6.0
                }
            } else {
                16.0
            }
        } else if is_video_playing && is_scrolling {
            self.upload_budget_ms * 0.6
        } else if is_video_playing {
            self.upload_budget_ms * 0.75
        } else if is_scrolling {
            if is_opengl {
                2.0
            } else if is_vulkan {
                4.0
            } else {
                self.upload_budget_ms * 0.85
            }
        } else {
            if is_vulkan {
                self.upload_budget_ms.min(8.0)
            } else {
                self.upload_budget_ms
            }
        };
        let upload_budget_ms = if is_burst {
            base_budget_ms
        } else {
            let max_budget_ms = if is_vulkan { 8.0 } else { 10.0 };
            (base_budget_ms * perf_scale).clamp(2.0, max_budget_ms)
        };
        let upload_budget = Duration::from_millis(upload_budget_ms.round() as u64);

        let selected_upload_path = self.selected_file.as_ref().map(|file| file.path.clone());
        let visible_upload_ranks = visible_thumbnail_upload_ranks(self);

        let visible_paths = if is_scrolling {
            eviction_visible.as_ref()
        } else {
            None
        };
        let mut deferred_count = 0;
        let offscreen_upload_budget = if is_scrolling {
            if is_performance_critical {
                0
            } else if is_performance_severe {
                1
            } else if is_opengl {
                1
            } else {
                2
            }
        } else {
            usize::MAX
        };
        let mut offscreen_uploads = 0usize;
        let discard_offscreen_pending = (is_opengl || is_vulkan) && is_scrolling;
        let max_offscreen_discards = max_uploads_per_frame.saturating_mul(8).max(8);
        let mut offscreen_discards = 0usize;

        while uploads_this_frame < max_uploads_per_frame {
            if upload_start.elapsed() >= upload_budget {
                break;
            }

            if let Some(next_upload_idx) = next_thumbnail_upload_index(
                &self.pending_thumbnails,
                selected_upload_path.as_ref(),
                &visible_upload_ranks,
            ) {
                let thumbnail_data = if next_upload_idx == 0 {
                    self.pending_thumbnails.pop_front()
                } else {
                    self.pending_thumbnails.remove(next_upload_idx)
                };
                let Some(thumbnail_data) = thumbnail_data else {
                    break;
                };

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
                        if discard_offscreen_pending {
                            self.cache_manager
                                .finish_pending_upload(&thumbnail_data.path);
                            offscreen_discards += 1;
                            if offscreen_discards >= max_offscreen_discards {
                                break;
                            }
                            continue;
                        }

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
                let premultiplied = thumbnail_data.premultiplied;
                let is_interactive = matches!(
                    thumbnail_data.priority,
                    crate::infrastructure::io_priority::IOPriority::Interactive
                );

                // Skip upload for paths pending deletion — the item was removed
                // externally and the texture would be immediately stale.
                if self
                    .file_operation_state
                    .pending_deletions
                    .contains_key(&path)
                {
                    self.cache_manager.finish_pending_upload(&path);
                    continue;
                }

                let is_selected = self
                    .selected_file
                    .as_ref()
                    .is_some_and(|selected_file| selected_file.path == path);

                // Compute preview minimum size before `path` is moved into cache operations.
                let preview_min_size_for_selected: u32 = if is_selected {
                    crate::domain::thumbnail::detail_preview_size(&path)
                } else {
                    0
                };

                let is_visible_or_selected = is_interactive
                    || is_selected
                    || eviction_visible
                        .as_ref()
                        .is_some_and(|visible_paths| visible_paths.contains(&path));
                if !is_visible_or_selected
                    && self.cache_manager.texture_cache.len()
                        >= self.cache_manager.texture_cache.cap().get()
                {
                    self.cache_manager.finish_pending_upload(&path);
                    continue;
                }

                let already_cached =
                    self.cache_manager
                        .texture_cache
                        .peek(&path)
                        .is_some_and(|texture| {
                            let size = texture.size();
                            let cached_dim = size[0].max(size[1]) as u32;
                            cached_dim >= width.max(height)
                        });
                if already_cached {
                    self.cache_manager
                        .thumbnail_trace
                        .record_upload_already_cached();
                    self.cache_manager.finish_pending_upload(&path);
                    continue;
                }

                let texture_name = path.to_string_lossy().into_owned();

                let color_image = if premultiplied {
                    egui::ColorImage::from_rgba_premultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    )
                } else {
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    )
                };

                let texture_upload_start = Instant::now();
                let texture =
                    ctx.load_texture(texture_name, color_image, egui::TextureOptions::LINEAR);
                log_slow_texture_upload(
                    "thumbnail",
                    texture_upload_start.elapsed(),
                    width,
                    height,
                    is_opengl,
                    is_vulkan,
                );

                self.cache_manager.thumbnail_trace.record_upload(&path);
                self.cache_manager.finish_pending_upload(&path);
                let should_cache_rgba = !is_vulkan
                    || is_selected
                    || eviction_visible
                        .as_ref()
                        .is_none_or(|visible_paths| visible_paths.contains(&path));
                if should_cache_rgba {
                    self.cache_manager.put_rgba_data(
                        path.clone(),
                        std::sync::Arc::clone(&rgba_data),
                        width,
                        height,
                    );
                }
                if let Some(visible_paths) = eviction_visible.as_ref() {
                    self.cache_manager.promote_visible(visible_paths);
                    self.cache_manager.put_thumbnail_preserving_visible(
                        path.clone(),
                        texture.clone(),
                        visible_paths,
                    );
                } else {
                    self.cache_manager
                        .put_thumbnail(path.clone(), texture.clone());
                }

                if is_selected {
                    let tex_dim = width.max(height);
                    // Only promote to selected_thumbnail when the uploaded texture
                    // meets the resolution the detail panel requires; otherwise a
                    // low-res upload from a smaller request can replace a pending
                    // high-res placeholder with a blurry thumbnail.
                    if tex_dim >= preview_min_size_for_selected {
                        self.selected_thumbnail = Some(texture);
                    } else if preview_min_size_for_selected > 0 {
                        // Best-effort: if we've already attempted the required quality
                        // bucket and the result is still smaller than ideal, accept it
                        // as the best available.  This happens with video files whose
                        // thumbnails cannot be extracted at higher resolutions.
                        let effective_req_size =
                            self.effective_thumbnail_request_size_px(preview_min_size_for_selected);
                        let required_bucket =
                            crate::workers::thumbnail::processing::get_bucket_size(
                                effective_req_size,
                            );
                        if self
                            .cache_manager
                            .attempted_thumbnail_bucket_for(&path)
                            .is_some_and(|bucket| bucket >= required_bucket)
                        {
                            if !self.cache_manager.best_effort_notified.contains(&path) {
                                self.cache_manager.best_effort_notified.insert(path.clone());
                                diag_info(
                                    "thumbnail_upload",
                                    "selected_best_effort",
                                    &[
                                        field_u64("tex_dim", tex_dim as u64),
                                        field_u64(
                                            "logical_req_size",
                                            preview_min_size_for_selected as u64,
                                        ),
                                        field_u64("effective_req_size", effective_req_size as u64),
                                        field_u64("required_bucket", required_bucket as u64),
                                    ],
                                );
                            }
                            self.selected_thumbnail = Some(texture);
                        }
                    }
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

        self.process_folder_preview_uploads(
            ctx,
            is_performance_critical,
            is_video_playing,
            is_burst,
            is_scrolling,
            eviction_visible.as_ref(),
        );
        received_any
    }

    fn should_skip_folder_cover_thumbnail_upload(&self, path: &PathBuf) -> bool {
        // Folder previews compose from the thumbnail disk cache directly.  The raw
        // cover thumbnail request is still useful as a readiness/retry signal for
        // unsafe media, but uploading that cover as its own grid texture creates a
        // redundant GPU upload wave in folders with many previewed subfolders.
        if self
            .selected_file
            .as_ref()
            .is_some_and(|selected| &selected.path == path)
        {
            return false;
        }

        if self.items.iter().any(|item| &item.path == path)
            || self.all_items.iter().any(|item| &item.path == path)
        {
            return false;
        }

        if self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| {
                let items = if snapshot.items_snapshot_compact {
                    snapshot.all_items.as_ref()
                } else {
                    snapshot.items.as_ref()
                };
                items.iter().any(|item| &item.path == path)
            })
        {
            return false;
        }

        self.all_items.iter().any(|item| {
            item.is_dir
                && item
                    .folder_cover
                    .as_ref()
                    .is_some_and(|cover| cover == path)
        }) || self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| {
                let items = if snapshot.items_snapshot_compact {
                    snapshot.all_items.as_ref()
                } else {
                    snapshot.items.as_ref()
                };
                items.iter().any(|item| {
                    item.is_dir
                        && item
                            .folder_cover
                            .as_ref()
                            .is_some_and(|cover| cover == path)
                })
            })
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
        let mut removed_folder_covers: Vec<PathBuf> = Vec::new();

        for item in self.all_items_mut().iter_mut() {
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
                removed_folder_covers.push(folder_path.clone());
                folders_to_refresh.insert(folder_path);
                updated_any = true;
                remaining_master = remaining_master.saturating_sub(1);
            }
        }

        for folder_path in &removed_folder_covers {
            self.app_state_db.remove_folder_cover(folder_path);
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
        is_burst: bool,
        is_scrolling: bool,
        visible_paths: Option<&crate::ui::cache::FxHashSet<PathBuf>>,
    ) {
        let is_opengl = self.is_opengl_backend();
        let is_vulkan = self.is_vulkan_backend();

        let max_folder_uploads: usize = if is_burst && is_opengl && is_scrolling {
            1
        } else if is_burst && is_opengl {
            2
        } else if is_burst && is_vulkan {
            if is_scrolling {
                2
            } else {
                6
            }
        } else if is_performance_critical {
            if is_opengl {
                1
            } else if is_vulkan {
                1
            } else {
                2
            }
        } else if is_opengl && is_scrolling {
            1
        } else if is_vulkan && is_scrolling {
            2
        } else if is_video_playing {
            if is_opengl {
                3
            } else if is_vulkan {
                4
            } else {
                6
            }
        } else if is_opengl {
            6
        } else if is_vulkan {
            6
        } else {
            20
        };

        // Time-budget folder preview uploads to avoid frame spikes.
        // Each ctx.load_texture() can take 5-15ms, so uncapped uploads
        // of 20 previews/frame could stall the UI for up to 300ms.
        // On OpenGL, each upload is synchronous and blocks the CPU thread,
        // so use a tighter budget to keep frames responsive.
        let budget = Duration::from_millis(if is_burst && is_opengl && is_scrolling {
            2
        } else if is_burst && is_opengl {
            2
        } else if is_burst && is_vulkan {
            4
        } else if is_performance_critical {
            2
        } else if is_opengl && is_scrolling {
            2
        } else if is_vulkan && is_scrolling {
            3
        } else if is_opengl {
            4
        } else if is_vulkan {
            4
        } else {
            8
        });
        let start = Instant::now();

        let mut folder_uploads = 0;
        let max_folder_results = if (is_opengl || is_vulkan) && is_scrolling {
            max_folder_uploads.saturating_mul(8).max(8)
        } else {
            max_folder_uploads
        };
        let mut processed_results = 0usize;

        while folder_uploads < max_folder_uploads && processed_results < max_folder_results {
            if folder_uploads > 0 && start.elapsed() >= budget {
                break;
            }
            if let Ok(data) = self.folder_preview_receiver.try_recv() {
                processed_results += 1;
                self.cache_manager.finish_folder_preview_loading(&data.path);
                let force_replace = self.pending_folder_preview_replace.remove(&data.path);

                if !self.is_folder_preview_result_relevant(&data.path) {
                    continue;
                }

                let offscreen_during_scroll_lod = (is_opengl || is_vulkan)
                    && is_scrolling
                    && !force_replace
                    && visible_paths.is_some_and(|visible| !visible.contains(&data.path));
                if offscreen_during_scroll_lod {
                    continue;
                }

                if !data.rgba_data.is_empty() {
                    let cached_size = self
                        .cache_manager
                        .folder_preview_cache
                        .peek(&data.path)
                        .map(|existing| existing.size());
                    match cached_size {
                        Some(size)
                            if size[0] >= data.width as usize
                                && size[1] >= data.height as usize
                                && !force_replace =>
                        {
                            continue;
                        }
                        Some(_) => {
                            self.cache_manager
                                .folder_preview_trace
                                .record_upload_size_diff();
                        }
                        None => {
                            self.cache_manager
                                .folder_preview_trace
                                .record_upload_no_cache();
                        }
                    }

                    let mut texture_name = String::from("folder_preview_");
                    texture_name.push_str(data.path.to_string_lossy().as_ref());
                    self.cache_manager.folder_preview_trace.record_upload();
                    let color_image = if data.premultiplied {
                        egui::ColorImage::from_rgba_premultiplied(
                            [data.width as usize, data.height as usize],
                            &data.rgba_data,
                        )
                    } else {
                        egui::ColorImage::from_rgba_unmultiplied(
                            [data.width as usize, data.height as usize],
                            &data.rgba_data,
                        )
                    };
                    let texture_upload_start = Instant::now();
                    let texture =
                        ctx.load_texture(texture_name, color_image, egui::TextureOptions::LINEAR);
                    log_slow_texture_upload(
                        "folder_preview",
                        texture_upload_start.elapsed(),
                        data.width,
                        data.height,
                        is_opengl,
                        is_vulkan,
                    );

                    if let Some(visible_paths) = visible_paths {
                        self.cache_manager.promote_visible(visible_paths);
                    }
                    self.cache_manager.put_folder_preview(data.path, texture);
                    folder_uploads += 1;
                }
            } else {
                break;
            }
        }

        if folder_uploads >= max_folder_uploads
            || processed_results >= max_folder_results
            || (folder_uploads > 0 && start.elapsed() >= budget)
        {
            ctx.request_repaint();
        }
    }

    fn is_folder_preview_result_relevant(&self, path: &PathBuf) -> bool {
        if self.items.iter().any(|item| &item.path == path) {
            return true;
        }

        if self.all_items.iter().any(|item| &item.path == path) {
            return true;
        }

        if self
            .selected_file
            .as_ref()
            .is_some_and(|selected| &selected.path == path)
        {
            return true;
        }

        if self
            .detail_panel_folder_preview_path()
            .is_some_and(|detail_path| detail_path.as_path() == path.as_path())
        {
            return true;
        }

        self.dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| {
                let items = if snapshot.items_snapshot_compact {
                    snapshot.all_items.as_ref()
                } else {
                    snapshot.items.as_ref()
                };
                items.iter().any(|item| &item.path == path)
            })
    }
}
