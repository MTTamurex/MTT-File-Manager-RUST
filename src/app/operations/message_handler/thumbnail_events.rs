use crate::app::state::{ImageViewerApp, ItemsRebuildResult};
use crate::application::sorting;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    fn build_sorted_items_snapshot(&self) -> Vec<crate::domain::file_entry::FileEntry> {
        let mut result_items = match sorting::filter_items_opt(&self.all_items, &self.search_query)
        {
            Some(filtered) => filtered,
            None => {
                let mut all = self.all_items.clone();
                sorting::sort_items(
                    &mut all,
                    self.sort_mode,
                    self.sort_descending,
                    self.folders_position,
                );
                all
            }
        };
        if !self.search_query.is_empty() {
            sorting::sort_items(
                &mut result_items,
                self.sort_mode,
                self.sort_descending,
                self.folders_position,
            );
        }
        result_items
    }

    pub(super) fn process_streaming_and_thumbnail_events(
        &mut self,
        ctx: &egui::Context,
    ) -> Instant {
        // 1. STREAMING: Recebe lotes incrementais de FileEntry (Filtrado por geração)
        // BLOCKING: Process all available file entries in batch

        // SAFETY TIMEOUT: Clear is_loading_folder if stuck for more than 30 seconds
        // This prevents infinite spinner if the loading thread fails silently
        if self.is_loading_folder && self.loading_started_at.elapsed().as_secs() > 30 {
            eprintln!("[FOLDER-LOADING] TIMEOUT: Loading took more than 30 seconds, clearing loading state");
            self.is_loading_folder = false;
        }

        let mut saw_end_of_load = false;
        loop {
            match self.file_entry_receiver.try_recv() {
                Ok((gen_id, new_batch)) => {
                    if gen_id != self.generation {
                        continue; // Descarta dados de uma navegação/refresh anterior
                    }

                    if new_batch.is_empty() {
                        // Lote vazio = Sinal de "Fim do Carregamento" da thread
                        saw_end_of_load = true;
                    } else {
                        // Chegou dados! Adiciona à lista mestre
                        self.pending_items_count =
                            self.pending_items_count.saturating_add(new_batch.len());
                        self.pending_items_rebuild = true;
                        self.all_items.extend(new_batch);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break, // No more messages
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        if saw_end_of_load {
            self.is_loading_folder = false;
            self.pending_deletions.clear();
            self.pending_items_rebuild = false;
            self.pending_items_count = 0;
            const INLINE_REBUILD_THRESHOLD: usize = 256;

            if self.all_items.len() <= INLINE_REBUILD_THRESHOLD {
                // Small folders: rebuild inline to avoid thread scheduling latency.
                let result_items = self.build_sorted_items_snapshot();
                self.items = Arc::new(result_items);
                self.total_items = self.items.len();

                if let Some(target_path) = self.pending_select_path.take() {
                    if let Some(idx) = self.items.iter().position(|i| i.path == target_path) {
                        self.selected_item = Some(idx);
                        self.selected_file = Some(self.items[idx].clone());
                        self.scroll_to_selected = true;
                    }
                }

                eprintln!(
                    "[PERF] Inline items rebuild (end-of-load): {} items",
                    self.total_items
                );
            } else {
                // Larger folders: keep rebuild off UI thread.
                self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
                let request_id = self.items_rebuild_request_id;
                let gen = self.generation;
                let items = self.all_items.clone();
                let query = self.search_query.clone();
                let sort_mode = self.sort_mode;
                let sort_descending = self.sort_descending;
                let folders_position = self.folders_position;
                let sender = self.items_rebuild_sender.clone();
                std::thread::spawn(move || {
                    let mut result_items = match sorting::filter_items_opt(&items, &query) {
                        Some(filtered) => filtered,
                        None => {
                            let mut all = items;
                            sorting::sort_items(
                                &mut all,
                                sort_mode,
                                sort_descending,
                                folders_position,
                            );
                            all
                        }
                    };
                    if !query.is_empty() {
                        sorting::sort_items(
                            &mut result_items,
                            sort_mode,
                            sort_descending,
                            folders_position,
                        );
                    }
                    let total = result_items.len();
                    let _ = sender.send(ItemsRebuildResult {
                        generation: gen,
                        request_id,
                        items: result_items,
                        total_items: total,
                    });
                });
            }

            // OneDrive folders: enqueue folder previews eagerly to reduce visual delay.
            if matches!(self.view_mode, crate::domain::file_entry::ViewMode::Grid)
                && !self.is_recycle_bin_view
                && crate::infrastructure::onedrive::is_onedrive_path(&PathBuf::from(
                    &self.current_path,
                ))
            {
                const MAX_EAGER_FOLDER_PREVIEWS: usize = 80;
                let eager_paths: Vec<PathBuf> = self
                    .all_items
                    .iter()
                    .filter(|i| i.is_dir && !i.is_archive())
                    .map(|i| i.path.clone())
                    .take(MAX_EAGER_FOLDER_PREVIEWS)
                    .collect();
                let mut queued = 0usize;
                for path in eager_paths {
                    if self.cache_manager.has_folder_preview(&path)
                        || self.cache_manager.is_folder_preview_loading(&path)
                    {
                        continue;
                    }
                    self.request_folder_preview_load(path);
                    queued += 1;
                }
                if queued > 0 {
                    eprintln!(
                        "[PERF] OneDrive eager folder preview queue: {} folders",
                        queued
                    );
                }
            }
            self.last_items_rebuild = Instant::now();
            ctx.request_repaint();
        } else if self.pending_items_rebuild {
            // Throttle rebuild para evitar sort a cada lote
            let elapsed = self.last_items_rebuild.elapsed();
            if elapsed > Duration::from_millis(80) || self.pending_items_count >= 1200 {
                self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
                let request_id = self.items_rebuild_request_id;
                let gen = self.generation;
                let items = self.all_items.clone();
                let query = self.search_query.clone();
                let sort_mode = self.sort_mode;
                let sort_descending = self.sort_descending;
                let folders_position = self.folders_position;
                let sender = self.items_rebuild_sender.clone();
                std::thread::spawn(move || {
                    let mut result_items = match sorting::filter_items_opt(&items, &query) {
                        Some(filtered) => filtered,
                        None => {
                            let mut all = items;
                            sorting::sort_items(
                                &mut all,
                                sort_mode,
                                sort_descending,
                                folders_position,
                            );
                            all
                        }
                    };
                    if !query.is_empty() {
                        sorting::sort_items(
                            &mut result_items,
                            sort_mode,
                            sort_descending,
                            folders_position,
                        );
                    }
                    let total = result_items.len();
                    let _ = sender.send(ItemsRebuildResult {
                        generation: gen,
                        request_id,
                        items: result_items,
                        total_items: total,
                    });
                });
                self.last_items_rebuild = Instant::now();
                self.pending_items_count = 0;
                self.pending_items_rebuild = false;
                ctx.request_repaint();
            }
        }

        // 2. Cover Worker: Recebe resultados de capas de folder
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Atualiza em all_items (fonte mutável)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    // PERFORMANCE: DB write moved to worker thread to avoid main thread stutter
                    folder_updates = true;

                    // Requisita thumbnail se necessário (Marcando como em carregamento para evitar loop)
                    if !self.cache_manager.has_thumbnail(&cover)
                        && self.cache_manager.start_loading(cover.clone())
                    {
                        self.request_thumbnail_load(cover, 256);
                    }
                }
            }
        }
        // Reconstrói items a partir de all_items se houve updates
        if folder_updates {
            self.filter_items();
            ctx.request_repaint();
        }

        let _t_streaming_done = Instant::now();

        // 3. Icon Worker: Recebe resultados de ícones assíncronos
        // PERFORMANCE: Throttle icon uploads - reduce when video is playing
        let max_icon_uploads = if self.is_video_playing_docked() { 2 } else { 5 };
        let mut icon_uploads = 0;
        while icon_uploads < max_icon_uploads {
            if let Ok((path, pixels, width, height)) = self.icon_res_receiver.try_recv() {
                self.loading_icons.remove(&path);

                // Skip texture creation if extraction failed (empty data)
                // Track failed icons to prevent infinite retry loops
                if pixels.is_empty() || width == 0 || height == 0 {
                    self.failed_icons.put(path, ());
                    icon_uploads += 1;
                    continue;
                }

                // Carrega textura no cache de ícones
                // FIX: Cache key must match icon_loader.rs format (path + size)
                // Icon worker uses IconSize::Jumbo for high-quality icons
                let cache_key = format!("{}_Jumbo", path.to_string_lossy());
                if !self.item_icon_loader.icon_cache.contains(&cache_key) {
                    let texture = ctx.load_texture(
                        cache_key.clone(),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [width as usize, height as usize],
                            &pixels,
                        ),
                        egui::TextureOptions::LINEAR,
                    );
                    self.item_icon_loader.icon_cache.put(cache_key, texture);
                }
                icon_uploads += 1;
            } else {
                break;
            }
        }
        if icon_uploads >= max_icon_uploads {
            ctx.request_repaint();
        }

        // 4. Metadata Worker: drena respostas mesmo sem thumbnails
        let mut metadata_updated = false;
        while let Ok((path, mtime, meta)) = self.metadata_res_receiver.try_recv() {
            self.metadata_loading.remove(&path);
            self.metadata_cache.put(path.clone(), (mtime, meta.clone()));

            if let Some(selected) = &self.selected_file {
                if selected.path == path {
                    self.selected_metadata = Some((path.clone(), meta));
                    metadata_updated = true;
                }
            }
        }
        if metadata_updated {
            ctx.request_repaint();
        }

        // 5. Individual thumbnails
        let mut received_any = false;

        // PERFORMANCE: Drain ALL pending thumbnails from worker into a persistent buffer
        // This ensures no data is lost when throttling GPU uploads.
        // PERFORMANCE: Limit pending_thumbnails buffer to prevent RAM spikes
        // Each thumbnail data can be ~1MB, so limit to ~64MB worth of pending data
        const MAX_PENDING_THUMBNAILS: usize = 64;

        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            // Se a imagem pertence a uma geração anterior (outra folder), descarta.
            if thumbnail_data.generation != self.generation {
                continue;
            }

            // Sempre libera o slot de loading, mesmo em falhas
            self.cache_manager.finish_loading(&thumbnail_data.path);

            // Se a imagem veio vazia, marca como falha para evitar retry infinito
            if thumbnail_data.image_data.is_empty() {
                self.cache_manager
                    .mark_as_failed(thumbnail_data.path.clone());

                // Stale folder cover cleanup: file was deleted from disk
                // Remove stale DB entry and re-discover a new cover asynchronously
                if thumbnail_data.not_found {
                    let failed = &thumbnail_data.path;
                    for item in self.all_items.iter_mut() {
                        if item.folder_cover.as_ref() == Some(failed) {
                            let folder = item.path.clone();
                            item.folder_cover = None;
                            self.disk_cache.remove_folder_cover(&folder);
                            let _ = self.cover_worker_sender.send(folder);
                        }
                    }
                }

                continue;
            }

            // PERFORMANCE: Drop oldest thumbnails if buffer is full
            // This prevents RAM spikes when workers produce faster than GPU upload
            while self.pending_thumbnails.len() >= MAX_PENDING_THUMBNAILS {
                if let Some(old) = self.pending_thumbnails.pop_front() {
                    self.cache_manager.finish_pending_upload(&old.path);
                }
            }

            // Adiciona ao buffer persistente para upload posterior
            self.cache_manager
                .start_pending_upload(thumbnail_data.path.clone());
            self.pending_thumbnails.push_back(thumbnail_data);
            received_any = true;
        }

        // PERFORMANCE: Adaptive GPU upload throttling based on scroll state AND video playback
        // Note: Thumbnail cache is on SSD, so we can be more generous with uploads
        let is_scrolling = self.last_scroll_time.elapsed() < std::time::Duration::from_millis(100);
        let is_video_playing = self.is_video_playing_docked();

        // CRITICAL PERFORMANCE MODE: Skip all non-essential uploads when FPS is critically low
        // This prevents compounding performance issues during heavy load
        const CRITICAL_FRAME_TIME_MS: f32 = 33.33; // < 30 FPS
        const SEVERE_FRAME_TIME_MS: f32 = 25.0; // < 40 FPS

        let is_performance_critical = self.frame_time_peak_ms > CRITICAL_FRAME_TIME_MS;
        let is_performance_severe = self.frame_time_peak_ms > SEVERE_FRAME_TIME_MS;

        let base_max_uploads = if is_performance_critical {
            1 // Minimal: only most essential uploads
        } else if is_performance_severe {
            2 // Reduced: critical performance mode
        } else if is_video_playing && is_scrolling {
            4 // Balanced: still load during scroll+video
        } else if is_scrolling {
            6 // Generous during scroll — time budget is the real limiter
        } else if is_video_playing {
            5 // Moderate limit during video
        } else {
            12 // Aggressive idle speed — fill visible area fast
        };
        let perf_scale = if self.frame_time_avg_ms <= 0.0 {
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
        let max_uploads_per_frame = ((base_max_uploads as f32) * perf_scale)
            .round()
            .clamp(1.0, 16.0) as usize;

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
                self.disk_cache
                    .set_preference("upload_budget_ms", &self.upload_budget_ms.to_string());
            }
            self.last_upload_budget_update = now;
        }

        let base_budget_ms = if is_video_playing && is_scrolling {
            self.upload_budget_ms * 0.6
        } else if is_video_playing {
            self.upload_budget_ms * 0.75
        } else if is_scrolling {
            self.upload_budget_ms * 0.85
        } else {
            self.upload_budget_ms
        };
        let upload_budget_ms = (base_budget_ms * perf_scale).clamp(2.0, 10.0);
        let upload_budget = Duration::from_millis(upload_budget_ms.round() as u64);

        // PERFORMANCE: Build set of visible item paths for upload prioritization
        // Uses cached set to avoid per-frame allocation during scroll
        // Only rebuilds when visible_index_range changes
        let visible_paths: Option<&crate::ui::cache::FxHashSet<PathBuf>> = if is_scrolling {
            // Check if we need to rebuild the cache
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

        // Process thumbnails from the buffer up to the per-frame limit
        while uploads_this_frame < max_uploads_per_frame {
            if let Some(thumbnail_data) = self.pending_thumbnails.pop_front() {
                if upload_start.elapsed() >= upload_budget {
                    self.pending_thumbnails.push_front(thumbnail_data);
                    break;
                }
                // Ensure thumbnail is still relevant (generation check again just in case)
                if thumbnail_data.generation != self.generation {
                    self.cache_manager
                        .finish_pending_upload(&thumbnail_data.path);
                    continue;
                }

                // PERFORMANCE: In critical mode, only process visible items
                // Skip non-visible uploads entirely to maintain responsiveness
                if is_performance_critical {
                    if let Some(vis) = visible_paths {
                        if !vis.contains(&thumbnail_data.path) {
                            // Defer to back of queue - will retry later when performance recovers
                            self.pending_thumbnails.push_back(thumbnail_data);
                            deferred_count += 1;
                            if deferred_count > max_uploads_per_frame * 2 {
                                break;
                            }
                            continue;
                        }
                    }
                }

                // PERFORMANCE: During scroll, prioritize visible items
                // Off-screen thumbnails are deferred to the back of the queue
                if let Some(vis) = visible_paths {
                    if !vis.contains(&thumbnail_data.path) {
                        self.pending_thumbnails.push_back(thumbnail_data);
                        deferred_count += 1;
                        // Safety limit: don't loop through entire queue
                        if deferred_count > max_uploads_per_frame * 3 {
                            break;
                        }
                        continue;
                    }
                }

                // PERFORMANCE: Extract RGBA data BEFORE moving to cache to avoid round-trip
                // Use local reference for GPU upload, then store in cache for future re-uploads
                let path = thumbnail_data.path.clone();
                let width = thumbnail_data.width;
                let height = thumbnail_data.height;
                let rgba_data = thumbnail_data.image_data; // Extract data before move

                // Carrega textura no GPU using local data (no cache lookup needed)
                let texture = ctx.load_texture(
                    path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                // Store RGBA data in RAM cache AFTER GPU upload for future re-uploads
                // This allows fast re-upload if texture is evicted from VRAM without disk I/O
                self.cache_manager
                    .put_rgba_data(path.clone(), rgba_data, width, height);

                self.cache_manager
                    .put_thumbnail(path.clone(), texture.clone());

                // Limpa status de pending upload
                self.cache_manager.finish_pending_upload(&path);

                // Update selected_thumbnail if it matches the selected_file
                if let Some(selected_file) = &self.selected_file {
                    if selected_file.path == path {
                        self.selected_thumbnail = Some(texture);
                    }
                }

                uploads_this_frame += 1;
                received_any = true;
            } else {
                break; // Buffer is empty
            }
        }

        // PERFORMANCE: Single repaint request after upload loop (not per-upload)
        if !self.pending_thumbnails.is_empty() {
            ctx.request_repaint();
        }

        // 6. Folder Previews (Native Sandwich effect)
        // PERFORMANCE: Throttle folder preview uploads (Max 2 per frame - heavy textures)
        let mut folder_uploads = 0;
        while folder_uploads < 2 {
            if let Ok(data) = self.folder_preview_receiver.try_recv() {
                self.cache_manager.finish_folder_preview_loading(&data.path);

                // Only create texture if we have actual data
                if !data.rgba_data.is_empty() {
                    let texture = ctx.load_texture(
                        format!("folder_preview_{}", data.path.to_string_lossy()),
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
        if folder_uploads >= 2 {
            ctx.request_repaint();
        }

        // 9. FOLDER SIZE RESULTS
        while let Ok(msg) = self.folder_size_res_receiver.try_recv() {
            match msg {
                crate::app::state::FolderSizeMessage::Progress {
                    folder_path,
                    total_size,
                } => {
                    self.folder_size_cache.put(folder_path, total_size);
                    received_any = true;
                }
                crate::app::state::FolderSizeMessage::Complete {
                    folder_path,
                    total_size,
                } => {
                    self.folder_size_loading.remove(&folder_path);
                    self.folder_size_cache.put(folder_path, total_size);
                    received_any = true;
                }
                crate::app::state::FolderSizeMessage::Cancelled { folder_path } => {
                    self.folder_size_loading.remove(&folder_path);
                    self.folder_size_cache.pop(&folder_path);
                    received_any = true;
                }
            }
        }

        if received_any {
            ctx.request_repaint();
        }

        _t_streaming_done
    }
}
