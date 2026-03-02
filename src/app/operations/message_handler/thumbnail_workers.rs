use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn process_cover_worker_results(&mut self, ctx: &egui::Context) {
        let t0 = Instant::now();

        // Cap per-frame processing to keep message handling responsive under heavy cover streams.
        const MAX_COVER_EVENTS_PER_FRAME: usize = 48;
        let mut cover_updates: std::collections::HashMap<std::path::PathBuf, Option<std::path::PathBuf>> =
            std::collections::HashMap::with_capacity(MAX_COVER_EVENTS_PER_FRAME);
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
        // Build a path index for master items and apply only touched updates.
        let mut all_items_index =
            std::collections::HashMap::with_capacity(self.all_items.len());
        for (idx, item) in self.all_items.iter().enumerate() {
            all_items_index.insert(item.path.clone(), idx);
        }
        for (folder_path, cover_opt) in &cover_updates {
            if let Some(idx) = all_items_index.get(folder_path) {
                let item = &mut self.all_items[*idx];
                if item.folder_cover != *cover_opt {
                    item.folder_cover = cover_opt.clone();
                    folder_updates = true;
                    covers_changed.push(folder_path.clone());
                }
            }
        }

        let t_all_items = Instant::now();

        // Build a path index for rendered items and apply only touched updates.
        let items = std::sync::Arc::make_mut(&mut self.items);
        let mut visible_items_index = std::collections::HashMap::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            visible_items_index.insert(item.path.clone(), idx);
        }
        for (folder_path, cover_opt) in &cover_updates {
            if let Some(idx) = visible_items_index.get(folder_path) {
                let item = &mut items[*idx];
                if item.folder_cover != *cover_opt {
                    item.folder_cover = cover_opt.clone();
                    folder_updates = true;
                }
            }
        }

        // When a folder's cover changes, the composed preview is stale —
        // invalidate it so the next frame triggers a fresh composition.
        for folder_path in &covers_changed {
            self.cache_manager.invalidate_folder_preview(folder_path);
        }

        let t_items = Instant::now();

        // Trigger thumbnail loads / cleanup once per updated folder.
        let mut none_count = 0usize;
        let mut load_count = 0usize;
        let mut folders_to_invalidate: Vec<std::path::PathBuf> = Vec::new();
        for (folder_path, cover_opt) in &cover_updates {
            match cover_opt {
                Some(cover) => {
                    if !self.cache_manager.has_thumbnail(cover)
                        && self.cache_manager.start_loading(cover.clone())
                    {
                        self.request_thumbnail_load(cover.clone(), 256);
                        load_count += 1;
                    }
                }
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
                "[PERF-COVERS] recv={}ms all_items={}ms arc_items={}ms trigger={}ms (updates={} loads={} removes={} all_items_len={} items_len={})",
                t_recv.duration_since(t0).as_millis(),
                t_all_items.duration_since(t_recv).as_millis(),
                t_items.duration_since(t_all_items).as_millis(),
                t_trigger.duration_since(t_items).as_millis(),
                cover_updates.len(),
                load_count,
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
                        if !pixels.is_empty() && width > 0 && height > 0 {
                            if let Some(ext) = path.extension() {
                                let ext_raw = ext.to_string_lossy().to_lowercase();
                                let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                                let ext_key = format!("{}_Large", ext_str);
                                if !self.item_icon_loader.extension_cache.contains_key(&ext_key) {
                                    let texture = ctx.load_texture(
                                        ext_key.clone(),
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [width as usize, height as usize],
                                            &pixels,
                                        ),
                                        egui::TextureOptions::LINEAR,
                                    );
                                    self.item_icon_loader.extension_cache.insert(ext_key, texture);
                                    prewarm_uploads += 1;
                                }
                            }
                            // Remove extension from loading set.
                            if let Some(ext) = path.extension() {
                                self.loading_extensions.remove(
                                    &ext.to_string_lossy().to_lowercase(),
                                );
                            }
                        }
                        continue; // Keep draining pre-warm results (within cap).
                    }
                    // Non-pre-warm result found — push back for Phase 2.
                    // We can't push back into mpsc, so process it inline.
                    self.process_single_icon_result(ctx, path, icon_generation, pixels, width, height);
                    phase1_processed_regular = true;
                    break; // Switch to budgeted Phase 2.
                }
                Err(_) => break, // Channel empty.
            }
        }

        // Phase 2: Process regular icon results with frame budget.
        let max_icon_uploads = if self.is_video_playing_docked() { 8 } else { 64 };
        let max_icon_messages = if self.is_video_playing_docked() { 48 } else { 256 };
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
                            let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                            let ext_key = format!("{}_Large", ext_str);
                            if !self.item_icon_loader.extension_cache.contains_key(&ext_key) {
                                let texture = ctx.load_texture(
                                    ext_key.clone(),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [width as usize, height as usize],
                                        &pixels,
                                    ),
                                    egui::TextureOptions::LINEAR,
                                );
                                self.item_icon_loader.extension_cache.insert(ext_key, texture);
                            }
                        }
                        if let Some(ext) = path.extension() {
                            self.loading_extensions.remove(
                                &ext.to_string_lossy().to_lowercase(),
                            );
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
            self.loading_extensions.remove(
                &ext.to_string_lossy().to_lowercase(),
            );
        }

        if pixels.is_empty() || width == 0 || height == 0 {
            self.failed_icons.put(path, ());
            return;
        }

        let path_text = path.to_string_lossy();
        let mut cache_key = String::with_capacity(path_text.len() + 6);
        cache_key.push_str(path_text.as_ref());
        cache_key.push_str("_Large");
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
            if let Some(ext) = path.extension() {
                let ext_raw = ext.to_string_lossy().to_lowercase();
                let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                if !matches!(ext_str, "exe" | "lnk" | "ico" | "cur" | "ani" | "com") {
                    let mut ext_key = String::with_capacity(ext_str.len() + 6);
                    ext_key.push_str(ext_str);
                    ext_key.push_str("_Large");
                    self.item_icon_loader
                        .extension_cache
                        .entry(ext_key)
                        .or_insert_with(|| texture.clone());
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
        let mut progress_updates: std::collections::HashMap<std::path::PathBuf, u64> =
            std::collections::HashMap::new();

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
                    total_size,
                } => {
                    // Coalesce multiple progress updates for the same folder into one cache write.
                    progress_updates.insert(folder_path, total_size);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Complete {
                    folder_path,
                    total_size,
                } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state.loading.remove(&folder_path);
                    self.folder_size_state.cache.put(folder_path, total_size);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Cancelled { folder_path } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state.loading.remove(&folder_path);
                    self.folder_size_state.cache.pop(&folder_path);
                    received_any = true;
                }
            }
        }

        for (folder_path, total_size) in progress_updates {
            self.folder_size_state.cache.put(folder_path, total_size);
        }

        if processed_messages >= MAX_FOLDER_SIZE_MSGS_PER_FRAME {
            has_more = true;
        }

        received_any || has_more
    }
}
