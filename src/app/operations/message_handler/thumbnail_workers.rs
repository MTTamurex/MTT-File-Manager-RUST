use crate::app::folder_size_state::FolderContentSummary;
use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn upsert_folder_content_summary(
    cache: &mut lru::LruCache<PathBuf, FolderContentSummary>,
    folder_path: PathBuf,
    summary: FolderContentSummary,
) {
    if let Some(existing) = cache.get_mut(&folder_path) {
        *existing = if summary.has_counts() {
            summary
        } else {
            existing.with_total_size(summary.total_size)
        };
    } else {
        cache.put(folder_path, summary);
    }
}

impl ImageViewerApp {
    pub(super) fn process_cover_worker_results(&mut self, ctx: &egui::Context) {
        let t0 = Instant::now();

        // Cap per-frame processing to keep message handling responsive under heavy cover streams.
        const MAX_COVER_EVENTS_PER_FRAME: usize = 48;
        let mut cover_updates: std::collections::HashMap<
            std::path::PathBuf,
            Option<std::path::PathBuf>,
        > = std::collections::HashMap::with_capacity(MAX_COVER_EVENTS_PER_FRAME);
        let mut processed = 0usize;
        let mut has_more = false;

        while processed < MAX_COVER_EVENTS_PER_FRAME {
            match self.cover_worker_receiver.try_recv() {
                Ok((folder_path, cover_opt)) => {
                    cover_updates.insert(folder_path, cover_opt);
                    processed += 1;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        let t_recv = Instant::now();

        if processed >= MAX_COVER_EVENTS_PER_FRAME {
            has_more = true;
        }

        if cover_updates.is_empty() {
            if has_more {
                ctx.request_repaint();
            }
            return;
        }

        let mut folder_updates = false;
        let mut covers_changed: Vec<std::path::PathBuf> = Vec::new();
        // Apply updates in-place without building temporary full-directory path indexes.
        for item in self.all_items_mut().iter_mut() {
            if let Some(cover_opt) = cover_updates.get(&item.path) {
                if item.folder_cover != *cover_opt {
                    // Only invalidate composed preview when cover PATH genuinely
                    // changed (Some(old) → Some(new)  or  Some(_) → None).
                    // The transition None → Some(path) is NOT a real change —
                    // it just fills in a field that DirectoryCache didn't have.
                    // The preview was already composed with this cover, so
                    // invalidating it causes a visible flash for no reason.
                    let cover_path_changed = match (&item.folder_cover, cover_opt) {
                        (Some(old), Some(new)) => old != new,
                        (Some(_), None) => true,
                        _ => false, // None→Some or None→None: not a real change
                    };
                    item.folder_cover = cover_opt.clone();
                    folder_updates = true;
                    if cover_path_changed {
                        covers_changed.push(item.path.clone());
                    }
                }
            }
        }

        let t_all_items = Instant::now();

        // Apply the same updates to the rendered snapshot without a second path index.
        let items = std::sync::Arc::make_mut(&mut self.items);
        for item in items.iter_mut() {
            if let Some(cover_opt) = cover_updates.get(&item.path) {
                if item.folder_cover != *cover_opt {
                    item.folder_cover = cover_opt.clone();
                    folder_updates = true;
                }
            }
        }

        // When a folder's cover changes, the composed preview is stale —
        // invalidate it so the next frame triggers a fresh composition.
        for folder_path in &covers_changed {
            if !self
                .suppress_next_folder_preview_invalidation
                .remove(folder_path)
            {
                self.cache_manager.invalidate_folder_preview(folder_path);
            }
        }

        for folder_path in cover_updates.keys() {
            self.suppress_next_folder_preview_invalidation
                .remove(folder_path);
        }

        let t_items = Instant::now();

        // Trigger cleanup once per updated folder. Folder previews compose from
        // the thumbnail disk cache directly, so loading raw cover textures here
        // only creates a redundant post-preview upload wave.
        let mut none_count = 0usize;
        let mut folders_to_invalidate: Vec<std::path::PathBuf> = Vec::new();
        for (folder_path, cover_opt) in &cover_updates {
            match cover_opt {
                Some(_) => {}
                None => {
                    folders_to_invalidate.push(folder_path.clone());
                    none_count += 1;
                }
            }
        }
        // Defer SQLite writes to background worker to avoid Mutex contention on UI thread.
        self.enqueue_disk_cache_invalidations(folders_to_invalidate);

        let t_trigger = Instant::now();
        let total_ms = t0.elapsed().as_millis();
        if total_ms > 20 {
            log::warn!(
                "[PERF-COVERS] recv={}ms all_items={}ms arc_items={}ms trigger={}ms (updates={} removes={} all_items_len={} items_len={})",
                t_recv.duration_since(t0).as_millis(),
                t_all_items.duration_since(t_recv).as_millis(),
                t_items.duration_since(t_all_items).as_millis(),
                t_trigger.duration_since(t_items).as_millis(),
                cover_updates.len(),
                none_count,
                self.all_items.len(),
                self.items.len(),
            );
        }

        if folder_updates || has_more {
            ctx.request_repaint();
        }
    }

    pub(super) fn process_icon_worker_results(&mut self, ctx: &egui::Context) {
        // Phase 1: Drain pre-warm results with a cap to prevent GPU upload storms (A-5).
        // Pre-warm results use usize::MAX generation and fake paths.
        // We only need to populate extension_cache, skip icon_cache.
        const MAX_PREWARM_UPLOADS_PER_FRAME: usize = 16;
        let mut phase1_processed_regular = false;
        let mut prewarm_uploads = 0usize;
        loop {
            if prewarm_uploads >= MAX_PREWARM_UPLOADS_PER_FRAME {
                // More pre-warm results may remain — continue next frame.
                ctx.request_repaint();
                break;
            }
            match self.icon_res_receiver.try_recv() {
                Ok((path, icon_generation, pixels, width, height)) => {
                    if icon_generation == usize::MAX {
                        // Pre-warm result: populate extension_cache only.
                        // Store under _Jumbo (primary) and _Large (backward compat).
                        if !pixels.is_empty() && width > 0 && height > 0 {
                            if let Some(ext) = path.extension() {
                                let ext_raw = ext.to_string_lossy().to_lowercase();
                                let ext_str =
                                    crate::infrastructure::windows::icons::canonical_icon_ext(
                                        &ext_raw,
                                    );
                                let ext_key_jumbo = format!("{}_Jumbo", ext_str);
                                let ext_key_large = format!("{}_Large", ext_str);
                                let need_jumbo = self
                                    .item_icon_loader
                                    .extension_cache
                                    .peek(&ext_key_jumbo)
                                    .is_none();
                                let need_large = self
                                    .item_icon_loader
                                    .extension_cache
                                    .peek(&ext_key_large)
                                    .is_none();
                                if need_jumbo || need_large {
                                    let texture = ctx.load_texture(
                                        ext_key_jumbo.clone(),
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [width as usize, height as usize],
                                            &pixels,
                                        ),
                                        egui::TextureOptions::LINEAR,
                                    );
                                    if need_jumbo {
                                        self.item_icon_loader
                                            .extension_cache
                                            .put(ext_key_jumbo, texture.clone());
                                    }
                                    if need_large {
                                        self.item_icon_loader
                                            .extension_cache
                                            .put(ext_key_large, texture);
                                    }
                                    prewarm_uploads += 1;
                                }
                            }
                            // Remove extension from loading set.
                            if let Some(ext) = path.extension() {
                                let ext_raw = ext.to_string_lossy().to_lowercase();
                                if !crate::infrastructure::windows::icons::is_per_file_icon_ext(
                                    &ext_raw,
                                ) {
                                    let ext_key =
                                        crate::infrastructure::windows::icons::canonical_icon_ext(
                                            &ext_raw,
                                        );
                                    self.loading_extensions.remove(ext_key);
                                }
                            }
                        }
                        continue; // Keep draining pre-warm results (within cap).
                    }
                    // Non-pre-warm result found — push back for Phase 2.
                    // We can't push back into mpsc, so process it inline.
                    self.process_single_icon_result(
                        ctx,
                        path,
                        icon_generation,
                        pixels,
                        width,
                        height,
                    );
                    phase1_processed_regular = true;
                    break; // Switch to budgeted Phase 2.
                }
                Err(_) => break, // Channel empty.
            }
        }

        // Phase 2: Process regular icon results with frame budget.
        let max_icon_uploads = if self.is_video_playing_docked() {
            8
        } else {
            64
        };
        let max_icon_messages = if self.is_video_playing_docked() {
            48
        } else {
            256
        };
        let icon_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(3)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(4)
        } else {
            Duration::from_millis(6)
        };
        let start = Instant::now();
        let mut icon_uploads = usize::from(phase1_processed_regular);
        let mut processed_messages = usize::from(phase1_processed_regular);
        let mut has_more = false;

        while processed_messages < max_icon_messages && icon_uploads < max_icon_uploads {
            if start.elapsed() >= icon_budget {
                has_more = true;
                break;
            }
            if let Ok((path, icon_generation, pixels, width, height)) =
                self.icon_res_receiver.try_recv()
            {
                processed_messages += 1;
                // Pre-warm that arrived during Phase 2 — handle eagerly.
                if icon_generation == usize::MAX {
                    if !pixels.is_empty() && width > 0 && height > 0 {
                        if let Some(ext) = path.extension() {
                            let ext_raw = ext.to_string_lossy().to_lowercase();
                            let ext_str =
                                crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                            let ext_key_jumbo = format!("{}_Jumbo", ext_str);
                            let ext_key_large = format!("{}_Large", ext_str);
                            let need_jumbo = self
                                .item_icon_loader
                                .extension_cache
                                .peek(&ext_key_jumbo)
                                .is_none();
                            let need_large = self
                                .item_icon_loader
                                .extension_cache
                                .peek(&ext_key_large)
                                .is_none();
                            if need_jumbo || need_large {
                                let texture = ctx.load_texture(
                                    ext_key_jumbo.clone(),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [width as usize, height as usize],
                                        &pixels,
                                    ),
                                    egui::TextureOptions::LINEAR,
                                );
                                if need_jumbo {
                                    self.item_icon_loader
                                        .extension_cache
                                        .put(ext_key_jumbo, texture.clone());
                                }
                                if need_large {
                                    self.item_icon_loader
                                        .extension_cache
                                        .put(ext_key_large, texture);
                                }
                            }
                        }
                        if let Some(ext) = path.extension() {
                            let ext_raw = ext.to_string_lossy().to_lowercase();
                            if !crate::infrastructure::windows::icons::is_per_file_icon_ext(
                                &ext_raw,
                            ) {
                                let ext_key =
                                    crate::infrastructure::windows::icons::canonical_icon_ext(
                                        &ext_raw,
                                    );
                                self.loading_extensions.remove(ext_key);
                            }
                        }
                    }
                    continue; // Don't count against budget.
                }
                self.process_single_icon_result(ctx, path, icon_generation, pixels, width, height);
                icon_uploads += 1;
            } else {
                break;
            }
        }

        if processed_messages >= max_icon_messages || icon_uploads >= max_icon_uploads {
            has_more = true;
        }

        if has_more {
            ctx.request_repaint();
        }
    }

    /// Process a single regular (non-pre-warm) icon result.
    fn process_single_icon_result(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        icon_generation: usize,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    ) {
        // Ignore stale icon results from previous folder generations.
        if icon_generation != self.generation {
            return;
        }

        self.loading_icons.remove(&path);
        // Remove extension from loading set.
        if let Some(ext) = path.extension() {
            let ext_raw = ext.to_string_lossy().to_lowercase();
            if !crate::infrastructure::windows::icons::is_per_file_icon_ext(&ext_raw) {
                let ext_key = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                self.loading_extensions.remove(ext_key);
            }
        }

        if pixels.is_empty() || width == 0 || height == 0 {
            self.failed_icons.put(path, ());
            return;
        }

        let path_text = path.to_string_lossy();
        // Store as Jumbo (256×256) — the async worker now extracts at Jumbo size.
        let mut cache_key = String::with_capacity(path_text.len() + 7);
        cache_key.push_str(path_text.as_ref());
        cache_key.push_str("_Jumbo");
        if !self.item_icon_loader.icon_cache.contains(&cache_key) {
            let texture = ctx.load_texture(
                cache_key.clone(),
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &pixels,
                ),
                egui::TextureOptions::LINEAR,
            );

            // Populate extension cache for instant icon sharing.
            // Store under both _Jumbo (primary, high-res) and _Large (backward compat)
            // so both get_or_load_icon_sized(Jumbo) and get_or_load_icon_sized(Large) hit.
            if let Some(ext) = path.extension() {
                let ext_raw = ext.to_string_lossy().to_lowercase();
                let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                if !crate::infrastructure::windows::icons::is_per_file_icon_ext(&ext_raw) {
                    let mut ext_key_jumbo = String::with_capacity(ext_str.len() + 7);
                    ext_key_jumbo.push_str(ext_str);
                    ext_key_jumbo.push_str("_Jumbo");
                    if self
                        .item_icon_loader
                        .extension_cache
                        .peek(&ext_key_jumbo)
                        .is_none()
                    {
                        self.item_icon_loader
                            .extension_cache
                            .put(ext_key_jumbo, texture.clone());
                    }
                    // Also seed _Large for callers that haven't migrated to Jumbo yet
                    let mut ext_key_large = String::with_capacity(ext_str.len() + 7);
                    ext_key_large.push_str(ext_str);
                    ext_key_large.push_str("_Large");
                    if self
                        .item_icon_loader
                        .extension_cache
                        .peek(&ext_key_large)
                        .is_none()
                    {
                        self.item_icon_loader
                            .extension_cache
                            .put(ext_key_large, texture.clone());
                    }
                }
            }

            self.item_icon_loader.icon_cache.put(cache_key, texture);
        }
    }

    pub(super) fn process_metadata_worker_results(&mut self, ctx: &egui::Context) {
        // PERF FIX (A-1): Cap + time budget to prevent stutter when many metadata
        // results arrive at once (e.g. after navigating to a large media folder).
        const MAX_METADATA_MSGS_PER_FRAME: usize = 32;
        let metadata_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(2)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(3)
        } else {
            Duration::from_millis(4)
        };
        let start = Instant::now();
        let mut metadata_updated = false;
        let mut processed = 0usize;
        let mut has_more = false;

        while processed < MAX_METADATA_MSGS_PER_FRAME {
            if start.elapsed() >= metadata_budget {
                has_more = true;
                break;
            }
            let Ok((path, mtime, meta)) = self.metadata_res_receiver.try_recv() else {
                break;
            };
            processed += 1;
            self.metadata_loading.remove(&path);
            self.metadata_cache.put(path.clone(), (mtime, meta.clone()));

            if let Some(selected) = &self.selected_file {
                if selected.path == path {
                    self.selected_metadata = Some((path.clone(), meta));
                    metadata_updated = true;
                }
            }
        }

        if processed >= MAX_METADATA_MSGS_PER_FRAME {
            has_more = true;
        }

        if metadata_updated || has_more {
            ctx.request_repaint();
        }
    }

    pub(super) fn process_live_file_size_worker_results(&mut self, ctx: &egui::Context) {
        const MAX_LIVE_SIZE_MSGS_PER_FRAME: usize = 64;
        let live_size_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(2)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(3)
        } else {
            Duration::from_millis(4)
        };

        let start = Instant::now();
        let mut processed = 0usize;
        let mut updated = false;
        let mut has_more = false;

        while processed < MAX_LIVE_SIZE_MSGS_PER_FRAME {
            if start.elapsed() >= live_size_budget {
                has_more = true;
                break;
            }

            let Ok((path, mtime, live_size)) = self.live_file_size_res_receiver.try_recv() else {
                break;
            };

            processed += 1;
            self.live_file_size_loading.remove(&path);

            if let Some(size) = live_size {
                self.live_file_size_cache.put(path, (mtime, size));
                updated = true;
            }
        }

        if processed >= MAX_LIVE_SIZE_MSGS_PER_FRAME {
            has_more = true;
        }

        if updated || has_more {
            ctx.request_repaint();
        }
    }

    fn process_deferred_panel_folder_size_revalidation(&mut self, now: Instant) -> bool {
        if self.selected_file.is_some()
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
            || self.file_operation_state.file_ops_in_progress > 0
            || self.is_loading_folder
            || self.frame_time_peak_ms > 25.0
        {
            return false;
        }

        let current_path = PathBuf::from(&self.navigation_state.current_path);
        if self.folder_size_state.loading.contains(&current_path) {
            return false;
        }
        if self
            .folder_size_state
            .cache
            .peek(&current_path)
            .is_some_and(|summary| summary.has_counts())
        {
            self.folder_size_state
                .clear_panel_stale_summary(&current_path);
            return false;
        }

        let Some(path) = self
            .folder_size_state
            .take_due_panel_revalidation(now, &current_path)
        else {
            return false;
        };

        self.folder_size_state.loading.insert(path.clone());
        let _ = self.folder_size_state.req_sender.send(path);
        true
    }

    pub(super) fn process_folder_size_results(&mut self) -> bool {
        const MAX_FOLDER_SIZE_MSGS_PER_FRAME: usize = 96;

        let folder_size_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(2)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(3)
        } else {
            Duration::from_millis(4)
        };

        let start = Instant::now();
        let mut received_any = false;
        let mut processed_messages = 0usize;
        let mut has_more = false;
        let mut progress_updates: std::collections::HashMap<
            std::path::PathBuf,
            FolderContentSummary,
        > = std::collections::HashMap::new();

        while processed_messages < MAX_FOLDER_SIZE_MSGS_PER_FRAME {
            if start.elapsed() >= folder_size_budget {
                has_more = true;
                break;
            }

            let msg = match self.folder_size_state.res_receiver.try_recv() {
                Ok(msg) => msg,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            };
            processed_messages += 1;

            match msg {
                crate::app::folder_size_state::FolderSizeMessage::Progress {
                    folder_path,
                    summary,
                } => {
                    // Coalesce multiple progress updates for the same folder into one cache write.
                    progress_updates.insert(folder_path, summary);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Complete {
                    folder_path,
                    summary,
                } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state.loading.remove(&folder_path);
                    self.folder_size_state
                        .clear_panel_stale_summary(&folder_path);
                    self.folder_size_state.cache.put(folder_path, summary);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Cancelled { folder_path } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state.loading.remove(&folder_path);
                    self.folder_size_state.cache.pop(&folder_path);
                    self.folder_size_state
                        .reschedule_panel_revalidation_if_stale(&folder_path, Instant::now());
                    received_any = true;
                }
            }
        }

        for (folder_path, summary) in progress_updates {
            upsert_folder_content_summary(&mut self.folder_size_state.cache, folder_path, summary);
        }

        if processed_messages >= MAX_FOLDER_SIZE_MSGS_PER_FRAME {
            has_more = true;
        }

        // ── Drain batch worker results (list-view folder sizes) ──
        {
            const MAX_BATCH_PER_FRAME: usize = 120;
            let mut batch_count = 0usize;
            while batch_count < MAX_BATCH_PER_FRAME {
                let result = match self.folder_size_state.batch_res_receiver.try_recv() {
                    Ok(r) => r,
                    Err(_) => break,
                };
                batch_count += 1;

                let crate::app::folder_size_state::BatchSizeResult {
                    folder_path,
                    total_size,
                    request_epoch,
                } = result;

                self.folder_size_state.batch_loading.remove(&folder_path);

                // Epoch-based staleness check: the result carries the epoch
                // that was active when its request was sent.  If a cache
                // invalidation bumped the epoch AFTER the request was sent,
                // the scan started with stale data — discard it.  The next
                // render will re-request a fresh scan.
                let current_epoch = self
                    .folder_size_state
                    .batch_invalidation_epoch
                    .get(&folder_path)
                    .copied()
                    .unwrap_or(0);
                if request_epoch < current_epoch {
                    // Stale result — discard.
                    received_any = true;
                    continue;
                }

                let Some(total_size) = total_size else {
                    // Service unavailable — keep in batch_loading and schedule
                    // a deferred retry to prevent per-frame re-requests.
                    self.folder_size_state
                        .batch_loading
                        .insert(folder_path.clone());
                    self.folder_size_state
                        .pending_revalidation
                        .entry(folder_path)
                        .or_insert_with(|| {
                            std::time::Instant::now() + std::time::Duration::from_secs(5)
                        });
                    received_any = true;
                    continue;
                };

                self.folder_size_state
                    .batch_cache
                    .put(folder_path.clone(), total_size);
                // Keep the preview-panel cache in sync so selecting the folder
                // in the details panel shows the same (fresh) value.
                upsert_folder_content_summary(
                    &mut self.folder_size_state.cache,
                    folder_path,
                    FolderContentSummary::size_only(total_size),
                );
                received_any = true;
            }
        }

        // ── Process deferred re-invalidations ──
        // Handles the timing race between client cache invalidation and
        // the search service's 2 s USN journal polling.  If a stale value
        // was re-cached before the service updated its index, this deferred
        // clear forces BOTH caches to re-fetch fresh data.
        //
        // Also bumps the invalidation epoch so any in-flight result that
        // was sent before the revalidation is discarded as stale.
        {
            let now = std::time::Instant::now();
            if self
                .folder_size_state
                .should_prune_pending_revalidations(now)
            {
                for path in self.folder_size_state.take_expired_revalidations(now) {
                    self.folder_size_state.pending_revalidation.remove(&path);
                    let is_current_folder_panel = self.selected_file.is_none()
                        && path == PathBuf::from(&self.navigation_state.current_path);
                    if is_current_folder_panel {
                        if let Some(summary) = self.folder_size_state.cache.peek(&path).copied() {
                            self.folder_size_state
                                .preserve_panel_summary_for_deferred_revalidation(
                                    path.clone(),
                                    summary,
                                    now,
                                );
                        } else {
                            self.folder_size_state
                                .reschedule_panel_revalidation_if_stale(&path, now);
                        }
                        if self.folder_size_state.panel_stale_cache.contains(&path) {
                            self.ui_ctx.request_repaint_after(
                                crate::app::folder_size_state::PANEL_STALE_REVALIDATION_DELAY
                                    + Duration::from_millis(25),
                            );
                        }
                    }
                    self.folder_size_state.batch_cache.pop(&path);
                    self.folder_size_state.batch_loading.remove(&path);
                    self.folder_size_state.cache.pop(&path);
                    self.folder_size_state.loading.remove(&path);
                    // Bump epoch so in-flight results from before are rejected.
                    *self
                        .folder_size_state
                        .batch_invalidation_epoch
                        .entry(path)
                        .or_insert(0) += 1;
                    received_any = true;
                }
            }

            if self.folder_size_state.should_prune_invalidation_epochs(now) {
                self.folder_size_state.prune_stale_invalidation_epochs(now);
            }

            received_any |= self.process_deferred_panel_folder_size_revalidation(now);
        }

        received_any || has_more
    }
}
