use crate::app::folder_size_state::FolderContentSummary;
use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn icon_size_suffix(size: crate::domain::file_entry::IconSize) -> &'static str {
    match size {
        crate::domain::file_entry::IconSize::Small => "Small",
        crate::domain::file_entry::IconSize::Large => "Large",
        crate::domain::file_entry::IconSize::Jumbo => "Jumbo",
    }
}

fn remove_loading_extension(
    loading_extensions: &mut rustc_hash::FxHashMap<String, u64>,
    key: &str,
    request_id: u64,
) {
    if loading_extensions.get(key) == Some(&request_id) {
        loading_extensions.remove(key);
    }
}

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

fn apply_folder_cover_updates_to_items(
    items: &mut [crate::domain::file_entry::FileEntry],
    cover_updates: &std::collections::HashMap<PathBuf, Option<PathBuf>>,
    covers_changed: &mut Vec<PathBuf>,
) -> bool {
    let mut folder_updates = false;
    for item in items.iter_mut() {
        if let Some(cover_opt) = cover_updates.get(&item.path) {
            if item.folder_cover != *cover_opt {
                // Only invalidate composed previews when the cover path genuinely
                // changed. None -> Some just fills metadata needed to request the
                // first preview and should not evict an existing composed preview.
                let cover_path_changed = match (&item.folder_cover, cover_opt) {
                    (Some(old), Some(new)) => old != new,
                    (Some(_), None) => true,
                    _ => false,
                };
                item.folder_cover = cover_opt.clone();
                folder_updates = true;
                if cover_path_changed && !covers_changed.iter().any(|path| path == &item.path) {
                    covers_changed.push(item.path.clone());
                }
            }
        }
    }
    folder_updates
}

fn apply_folder_cover_updates_to_snapshot_items(
    items: &mut Arc<Vec<crate::domain::file_entry::FileEntry>>,
    items_revision: &mut u64,
    cover_updates: &std::collections::HashMap<PathBuf, Option<PathBuf>>,
    covers_changed: &mut Vec<PathBuf>,
) -> bool {
    let changed = apply_folder_cover_updates_to_items(
        Arc::make_mut(items).as_mut_slice(),
        cover_updates,
        covers_changed,
    );
    if changed {
        *items_revision = items_revision.wrapping_add(1);
    }
    changed
}

impl ImageViewerApp {
    pub(super) fn process_cover_worker_results(&mut self, ctx: &egui::Context) {
        let t0 = Instant::now();

        // Cap per-frame processing to keep message handling responsive under heavy cover streams.
        const MAX_COVER_EVENTS_PER_FRAME: usize = 48;
        const CONSERVATIVE_MAX_COVER_EVENTS_PER_FRAME: usize = 12;
        let max_cover_events_per_frame = if self.uses_conservative_folder_preview_policy() {
            CONSERVATIVE_MAX_COVER_EVENTS_PER_FRAME
        } else {
            MAX_COVER_EVENTS_PER_FRAME
        };
        let mut cover_updates: std::collections::HashMap<
            std::path::PathBuf,
            Option<std::path::PathBuf>,
        > = std::collections::HashMap::with_capacity(max_cover_events_per_frame);
        let mut processed = 0usize;
        let mut has_more = false;

        while processed < max_cover_events_per_frame {
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

        if processed >= max_cover_events_per_frame {
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
        folder_updates |= apply_folder_cover_updates_to_items(
            self.all_items_mut().as_mut_slice(),
            &cover_updates,
            &mut covers_changed,
        );

        let t_all_items = Instant::now();

        // Apply the same updates to the rendered snapshot without a second path index.
        let items = std::sync::Arc::make_mut(&mut self.items);
        folder_updates |= apply_folder_cover_updates_to_items(
            items.as_mut_slice(),
            &cover_updates,
            &mut covers_changed,
        );

        // The cover worker is shared by both visible panes. When the unfocused
        // pane is a tag view, its FileEntry values are stored only in the
        // inactive snapshot; without updating it here, folder previews never get
        // requested until a focused regular folder populates the cover cache.
        if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
            folder_updates |= apply_folder_cover_updates_to_snapshot_items(
                &mut snapshot.all_items,
                &mut snapshot.items_revision,
                &cover_updates,
                &mut covers_changed,
            );

            if !snapshot.items_snapshot_compact {
                let inactive_items = std::sync::Arc::make_mut(&mut snapshot.items);
                folder_updates |= apply_folder_cover_updates_to_items(
                    inactive_items.as_mut_slice(),
                    &cover_updates,
                    &mut covers_changed,
                );
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

        let preview_requests: Vec<_> = if self.uses_conservative_folder_preview_policy() {
            // Avoid a background composition burst. Visible slots request previews
            // on demand so work remains bounded as large folder/tag views load.
            Vec::new()
        } else {
            cover_updates
                .iter()
                .filter_map(|(folder_path, cover_opt)| {
                    let cover_path = cover_opt.as_ref()?;
                    if self.cache_manager.has_folder_preview(folder_path)
                        || self.cache_manager.is_folder_preview_loading(folder_path)
                        || !self.is_folder_preview_result_relevant(folder_path)
                    {
                        return None;
                    }
                    Some((folder_path.clone(), cover_path.clone()))
                })
                .collect()
        };

        for (folder_path, cover_path) in preview_requests {
            if !self
                .cache_manager
                .start_folder_preview_loading(folder_path.clone())
            {
                continue;
            }

            let request = crate::workers::folder_preview_worker::FolderPreviewRequest::new(
                folder_path.clone(),
                self.effective_folder_preview_request_size_px(),
                Some(cover_path),
            );
            match self.folder_preview_sender.try_send(request) {
                Ok(()) => self
                    .cache_manager
                    .note_folder_preview_request_sent(&folder_path),
                Err(err) => {
                    let request = err.into_inner();
                    self.cache_manager
                        .finish_folder_preview_loading(&request.path);
                }
            }
        }

        let t_items = Instant::now();

        // Trigger cleanup once per updated folder. Folder previews compose from
        // the thumbnail disk cache directly, so loading raw cover textures here
        // only creates a redundant post-preview upload wave.
        let mut none_count = 0usize;
        let mut folders_to_remove: Vec<std::path::PathBuf> = Vec::new();
        for (folder_path, cover_opt) in &cover_updates {
            match cover_opt {
                Some(_) => {}
                None => {
                    folders_to_remove.push(folder_path.clone());
                    none_count += 1;
                }
            }
        }
        self.enqueue_disk_cache_invalidations(folders_to_remove.clone());
        for folder_path in folders_to_remove {
            self.remove_folder_cover_without_blocking(&folder_path, false);
        }

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
        // Drain-scoped lazy cache of inactive-panel paths, shared across every
        // process_single_icon_result call in this drain (built at most once).
        let mut inactive_panel_paths: Option<Option<crate::ui::cache::FxHashSet<PathBuf>>> = None;
        loop {
            if prewarm_uploads >= MAX_PREWARM_UPLOADS_PER_FRAME {
                // More pre-warm results may remain — continue next frame.
                ctx.request_repaint();
                break;
            }
            match self.icon_res_receiver.try_recv() {
                Ok((path, icon_generation, icon_size, request_id, pixels, width, height)) => {
                    if icon_generation == usize::MAX {
                        // Pre-warm result: populate extension_cache only.
                        if crate::domain::thumbnail::is_valid_rgba_buffer(
                            width,
                            height,
                            crate::domain::thumbnail::MAX_ICON_SIDE,
                            pixels.len(),
                        ) {
                            if let Some(ext) = path.extension() {
                                let ext_raw = ext.to_string_lossy().to_lowercase();
                                let ext_str =
                                    crate::infrastructure::windows::icons::canonical_icon_ext(
                                        &ext_raw,
                                    );
                                let ext_key =
                                    format!("{}_{}", ext_str, icon_size_suffix(icon_size));
                                if self
                                    .item_icon_loader
                                    .extension_cache
                                    .peek(&ext_key)
                                    .is_none()
                                {
                                    let texture = ctx.load_texture(
                                        ext_key.clone(),
                                        egui::ColorImage::from_rgba_unmultiplied(
                                            [width as usize, height as usize],
                                            &pixels,
                                        ),
                                        egui::TextureOptions::LINEAR,
                                    );
                                    self.item_icon_loader.extension_cache.put(ext_key, texture);
                                    prewarm_uploads += 1;
                                }
                            }
                            // Remove extension from loading set.
                            if let Some(ext) = path.extension() {
                                let ext_raw = ext.to_string_lossy().to_lowercase();
                                if !crate::infrastructure::windows::icons::is_per_file_icon_ext(
                                    &ext_raw,
                                ) {
                                    let ext_key = format!(
                                        "{}_{}",
                                        crate::infrastructure::windows::icons::canonical_icon_ext(
                                            &ext_raw,
                                        ),
                                        icon_size_suffix(icon_size)
                                    );
                                    remove_loading_extension(
                                        &mut self.loading_extensions,
                                        &ext_key,
                                        request_id,
                                    );
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
                        icon_size,
                        request_id,
                        pixels,
                        width,
                        height,
                        &mut inactive_panel_paths,
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
            if let Ok((path, icon_generation, icon_size, request_id, pixels, width, height)) =
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
                            let ext_key = format!("{}_{}", ext_str, icon_size_suffix(icon_size));
                            if self
                                .item_icon_loader
                                .extension_cache
                                .peek(&ext_key)
                                .is_none()
                            {
                                let texture = ctx.load_texture(
                                    ext_key.clone(),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [width as usize, height as usize],
                                        &pixels,
                                    ),
                                    egui::TextureOptions::LINEAR,
                                );
                                self.item_icon_loader.extension_cache.put(ext_key, texture);
                            }
                        }
                        if let Some(ext) = path.extension() {
                            let ext_raw = ext.to_string_lossy().to_lowercase();
                            if !crate::infrastructure::windows::icons::is_per_file_icon_ext(
                                &ext_raw,
                            ) {
                                let ext_key = format!(
                                    "{}_{}",
                                    crate::infrastructure::windows::icons::canonical_icon_ext(
                                        &ext_raw,
                                    ),
                                    icon_size_suffix(icon_size)
                                );
                                remove_loading_extension(
                                    &mut self.loading_extensions,
                                    &ext_key,
                                    request_id,
                                );
                            }
                        }
                    }
                    continue; // Don't count against budget.
                }
                self.process_single_icon_result(
                    ctx,
                    path,
                    icon_generation,
                    icon_size,
                    request_id,
                    pixels,
                    width,
                    height,
                    &mut inactive_panel_paths,
                );
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
    ///
    /// `inactive_panel_paths` is a drain-scoped lazy cache of the inactive
    /// panel's paths; it is built at most once per `process_icon_worker_results`
    /// call, on the first result whose generation diverges.
    #[allow(clippy::too_many_arguments)]
    fn process_single_icon_result(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        icon_generation: usize,
        icon_size: crate::domain::file_entry::IconSize,
        request_id: u64,
        pixels: std::sync::Arc<[u8]>,
        width: u32,
        height: u32,
        inactive_panel_paths: &mut Option<Option<crate::ui::cache::FxHashSet<PathBuf>>>,
    ) {
        let is_current_request = self.loading_icons.remove(&path, icon_size, request_id);
        let shared_ext_key = path.extension().and_then(|ext| {
            let ext_raw = ext.to_string_lossy().to_lowercase();
            (!crate::infrastructure::windows::icons::is_per_file_icon_ext(&ext_raw)).then(|| {
                format!(
                    "{}_{}",
                    crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw),
                    icon_size_suffix(icon_size)
                )
            })
        });
        // Remove the extension from the loading set BEFORE any generation or
        // validity discard, so a stale/failed/panicked response never leaves the
        // extension marker stuck and blocking retries.
        if let Some(ext_key) = shared_ext_key.as_deref() {
            remove_loading_extension(&mut self.loading_extensions, ext_key, request_id);
        }
        if !is_current_request {
            return;
        }

        // Ignore stale icon results from previous folder generations unless
        // the path still belongs to the visible inactive dual-panel snapshot.
        if icon_generation != self.generation
            && !inactive_panel_paths
                .get_or_insert_with(|| self.collect_inactive_panel_paths())
                .as_ref()
                .is_some_and(|set| set.contains(&path))
        {
            return;
        }

        // Validate the buffer before ColorImage::from_rgba_*. Only poison
        // failed_icons for a result that is relevant to the CURRENT generation;
        // an empty terminal response used to release markers for a stale or
        // panicked request must not suppress future retries.
        if !crate::domain::thumbnail::is_valid_rgba_buffer(
            width,
            height,
            crate::domain::thumbnail::MAX_ICON_SIDE,
            pixels.len(),
        ) {
            if icon_generation == self.generation {
                if let Some(ext_key) = shared_ext_key {
                    self.failed_extensions.put(ext_key, Instant::now());
                } else {
                    self.failed_icons.insert(path, icon_size);
                }
            }
            return;
        }

        if let Some(ext_key) = shared_ext_key.as_deref() {
            self.failed_extensions.pop(ext_key);
        }

        let path_text = path.to_string_lossy();
        let size_suffix = icon_size_suffix(icon_size);
        let mut cache_key = String::with_capacity(path_text.len() + size_suffix.len() + 1);
        cache_key.push_str(path_text.as_ref());
        cache_key.push('_');
        cache_key.push_str(size_suffix);
        if !self.item_icon_loader.icon_cache.contains(&cache_key) {
            let texture = ctx.load_texture(
                cache_key.clone(),
                egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &pixels,
                ),
                egui::TextureOptions::LINEAR,
            );

            // Populate the extension cache for callers requesting this exact size.
            if let Some(ext) = path.extension() {
                let ext_raw = ext.to_string_lossy().to_lowercase();
                let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_raw);
                if !crate::infrastructure::windows::icons::is_per_file_icon_ext(&ext_raw) {
                    let mut ext_key = String::with_capacity(ext_str.len() + size_suffix.len() + 1);
                    ext_key.push_str(ext_str);
                    ext_key.push('_');
                    ext_key.push_str(size_suffix);
                    if self
                        .item_icon_loader
                        .extension_cache
                        .peek(&ext_key)
                        .is_none()
                    {
                        self.item_icon_loader
                            .extension_cache
                            .put(ext_key, texture.clone());
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
            if crate::infrastructure::onedrive::is_cloud_sync_path(&path)
                && meta.is_empty()
                && !crate::infrastructure::onedrive::is_locally_available(&path)
            {
                if let Some(selected) = &self.selected_file {
                    if selected.path == path {
                        self.selected_metadata = None;
                    }
                }
                continue;
            }
            self.metadata_cache.put(path.clone(), (mtime, meta.clone()));

            if let Some(selected) = &self.selected_file {
                if selected.path == path && (mtime == 0 || selected.modified == mtime) {
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

            let Ok(response) = self.live_file_size_res_receiver.try_recv() else {
                break;
            };

            processed += 1;
            if let Some(next_revalidation) =
                crate::app::live_file_size::accept_live_file_size_response(
                    response,
                    &mut self.live_file_size_cache,
                    &mut self.live_file_size_loading,
                    Instant::now(),
                )
            {
                updated = true;
                ctx.request_repaint_after(next_revalidation);
            }
        }

        if processed >= MAX_LIVE_SIZE_MSGS_PER_FRAME {
            has_more = true;
        }

        if updated || has_more {
            ctx.request_repaint();
        }
    }

    pub(super) fn process_file_hash_worker_results(&mut self, ctx: &egui::Context) {
        const MAX_HASH_MSGS_PER_FRAME: usize = 4;
        let hash_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(1)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(2)
        } else {
            Duration::from_millis(3)
        };
        let start = Instant::now();
        let mut processed = 0usize;
        let mut updated = false;
        let mut has_more = false;

        while processed < MAX_HASH_MSGS_PER_FRAME {
            if start.elapsed() >= hash_budget {
                has_more = true;
                break;
            }
            let Ok((path, modified, size, result)) = self.file_hash_res_receiver.try_recv() else {
                break;
            };
            processed += 1;
            self.file_hash_loading.remove(&path);
            if let Err(msg) = &result {
                log::debug!("[FileHash] failed for {:?}: {}", path, msg);
            }

            let is_current_selection = self.selected_file.as_ref().is_some_and(|file| {
                let status = crate::app::file_hash::file_hash_status(file);
                file.path == path && status.modified == modified && status.size == size
            });

            if is_current_selection {
                self.selected_file_hash = Some((path, modified, size, result));
                updated = true;
            }
        }

        if processed >= MAX_HASH_MSGS_PER_FRAME {
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

        let request_epoch = self
            .folder_size_state
            .batch_invalidation_epoch
            .get(&path)
            .copied()
            .unwrap_or(0);
        self.folder_size_state
            .dispatch_request(path, request_epoch, now)
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
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    received_any |= !self
                        .folder_size_state
                        .fail_all_requests(Instant::now())
                        .is_empty();
                    break;
                }
            };
            processed_messages += 1;

            let (message_path, request_epoch, request_id, terminal) = match &msg {
                crate::app::folder_size_state::FolderSizeMessage::Progress {
                    folder_path,
                    request_epoch,
                    request_id,
                    ..
                } => (folder_path, *request_epoch, *request_id, false),
                crate::app::folder_size_state::FolderSizeMessage::Complete {
                    folder_path,
                    request_epoch,
                    request_id,
                    ..
                }
                | crate::app::folder_size_state::FolderSizeMessage::Cancelled {
                    folder_path,
                    request_epoch,
                    request_id,
                }
                | crate::app::folder_size_state::FolderSizeMessage::Failed {
                    folder_path,
                    request_epoch,
                    request_id,
                } => (folder_path, *request_epoch, *request_id, true),
            };
            if !self
                .folder_size_state
                .is_active_request(message_path, request_id)
            {
                received_any = true;
                continue;
            }
            let current_epoch = self
                .folder_size_state
                .batch_invalidation_epoch
                .get(message_path)
                .copied()
                .unwrap_or(0);
            if request_epoch < current_epoch {
                if terminal {
                    self.folder_size_state
                        .finish_request(message_path, request_id);
                    self.folder_size_state.cache.pop(message_path);
                    self.folder_size_state
                        .reschedule_panel_revalidation_if_stale(message_path, Instant::now());
                }
                received_any = true;
                continue;
            }

            match msg {
                crate::app::folder_size_state::FolderSizeMessage::Progress {
                    folder_path,
                    summary,
                    request_epoch: _,
                    request_id: _,
                } => {
                    // Coalesce multiple progress updates for the same folder into one cache write.
                    progress_updates.insert(folder_path, summary);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Complete {
                    folder_path,
                    summary,
                    request_epoch: _,
                    request_id,
                } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state
                        .cancel_revalidation_if_changed(&folder_path, summary.total_size);
                    self.folder_size_state
                        .finish_request(&folder_path, request_id);
                    self.folder_size_state
                        .clear_panel_stale_summary(&folder_path);
                    self.folder_size_state.clear_failure(&folder_path);
                    *self
                        .folder_size_state
                        .batch_invalidation_epoch
                        .entry(folder_path.clone())
                        .or_insert(0) += 1;
                    self.folder_size_state
                        .batch_cache
                        .put(folder_path.clone(), summary.total_size);
                    self.folder_size_state.cache.put(folder_path, summary);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Cancelled {
                    folder_path,
                    request_epoch: _,
                    request_id,
                } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state
                        .finish_request(&folder_path, request_id);
                    self.folder_size_state.cache.pop(&folder_path);
                    self.folder_size_state
                        .reschedule_panel_revalidation_if_stale(&folder_path, Instant::now());
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Failed {
                    folder_path,
                    request_epoch: _,
                    request_id,
                } => {
                    progress_updates.remove(&folder_path);
                    self.folder_size_state
                        .finish_request(&folder_path, request_id);
                    self.folder_size_state.cache.pop(&folder_path);
                    self.folder_size_state
                        .record_failure(folder_path, Instant::now());
                    received_any = true;
                }
            }
        }

        for (folder_path, summary) in progress_updates {
            self.folder_size_state
                .batch_cache
                .put(folder_path.clone(), summary.total_size);
            upsert_folder_content_summary(&mut self.folder_size_state.cache, folder_path, summary);
        }

        if processed_messages >= MAX_FOLDER_SIZE_MSGS_PER_FRAME {
            has_more = true;
        }
        if !has_more
            && !self
                .folder_size_state
                .expire_requests(Instant::now())
                .is_empty()
        {
            received_any = true;
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
                    let delay = self
                        .folder_size_state
                        .schedule_revalidation_if_absent(folder_path, std::time::Instant::now());
                    self.ui_ctx
                        .request_repaint_after(delay + Duration::from_millis(25));
                    received_any = true;
                    continue;
                };

                self.folder_size_state
                    .cancel_revalidation_if_changed(&folder_path, total_size);

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
        // was re-cached before the service updated its index, this delayed
        // pass clears BOTH caches for one fresh fetch.
        //
        // Also bumps the invalidation epoch so any in-flight result that
        // was sent before the revalidation is discarded as stale.
        {
            let now = std::time::Instant::now();
            if self
                .folder_size_state
                .should_prune_pending_revalidations(now)
            {
                for (path, release_batch_loading) in
                    self.folder_size_state.take_expired_revalidations(now)
                {
                    let is_current_folder_panel = self.selected_file.is_none()
                        && path.as_path() == Path::new(&self.navigation_state.current_path);
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
                    if release_batch_loading {
                        self.folder_size_state.batch_loading.remove(&path);
                    }
                    self.folder_size_state.cache.pop(&path);
                    self.folder_size_state.cancel_active_request(&path);
                    self.folder_size_state.clear_failure(&path);
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

#[cfg(test)]
mod tests {
    use super::apply_folder_cover_updates_to_snapshot_items;
    use crate::domain::file_entry::{FileEntry, SyncStatus};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn folder(cover: Option<&str>) -> FileEntry {
        FileEntry {
            path: PathBuf::from("folder"),
            name: "folder".into(),
            is_dir: true,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: cover.map(PathBuf::from),
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn inactive_cover_update_increments_revision_only_when_all_items_changes() {
        let mut items = Arc::new(vec![folder(None)]);
        let mut revision = 4;
        let mut changed_paths = Vec::new();
        let updates = HashMap::from([(
            PathBuf::from("folder"),
            Some(PathBuf::from("folder/cover.jpg")),
        )]);

        assert!(apply_folder_cover_updates_to_snapshot_items(
            &mut items,
            &mut revision,
            &updates,
            &mut changed_paths,
        ));
        assert_eq!(revision, 5);

        assert!(!apply_folder_cover_updates_to_snapshot_items(
            &mut items,
            &mut revision,
            &updates,
            &mut changed_paths,
        ));
        assert_eq!(revision, 5);
    }
}
