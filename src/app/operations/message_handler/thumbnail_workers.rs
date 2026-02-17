use crate::app::state::ImageViewerApp;
use eframe::egui;
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

        // Apply updates to master list.
        for item in self.all_items.iter_mut() {
            if let Some(cover_opt) = cover_updates.get(&item.path) {
                if item.folder_cover != *cover_opt {
                    item.folder_cover = cover_opt.clone();
                    folder_updates = true;
                }
            }
        }

        let t_all_items = Instant::now();

        // Apply updates to currently rendered list without full filter/sort rebuild.
        let items = std::sync::Arc::make_mut(&mut self.items);
        for item in items.iter_mut() {
            if let Some(cover_opt) = cover_updates.get(&item.path) {
                if item.folder_cover != *cover_opt {
                    item.folder_cover = cover_opt.clone();
                    folder_updates = true;
                }
            }
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
        let max_icon_uploads = if self.is_video_playing_docked() { 8 } else { 32 };
        let max_icon_messages = if self.is_video_playing_docked() { 48 } else { 128 };
        let icon_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(2)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(3)
        } else {
            Duration::from_millis(5)
        };
        let start = Instant::now();
        let mut icon_uploads = 0;
        let mut processed_messages = 0usize;
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
                // Ignore stale icon results from previous folder generations.
                if icon_generation != self.generation {
                    continue;
                }

                self.loading_icons.remove(&path);

                if pixels.is_empty() || width == 0 || height == 0 {
                    self.failed_icons.put(path, ());
                    icon_uploads += 1;
                    continue;
                }

                let cache_key = format!("{}_Large", path.to_string_lossy());
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

        if processed_messages >= max_icon_messages || icon_uploads >= max_icon_uploads {
            has_more = true;
        }

        if has_more {
            ctx.request_repaint();
        }
    }

    pub(super) fn process_metadata_worker_results(&mut self, ctx: &egui::Context) {
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
