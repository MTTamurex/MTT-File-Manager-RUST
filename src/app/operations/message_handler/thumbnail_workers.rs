use crate::app::state::ImageViewerApp;
use eframe::egui;

impl ImageViewerApp {
    pub(super) fn process_cover_worker_results(&mut self, ctx: &egui::Context) {
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                match cover_opt {
                    Some(cover) => {
                        if item.folder_cover.as_ref() != Some(&cover) {
                            item.folder_cover = Some(cover.clone());
                            folder_updates = true;
                        }

                        if !self.cache_manager.has_thumbnail(&cover)
                            && self.cache_manager.start_loading(cover.clone())
                        {
                            self.request_thumbnail_load(cover, 256);
                        }
                    }
                    None => {
                        if item.folder_cover.take().is_some() {
                            self.disk_cache.remove_folder_cover(&folder_path);
                            folder_updates = true;
                        }
                    }
                }
            }
        }

        if folder_updates {
            self.filter_items();
            ctx.request_repaint();
        }
    }

    pub(super) fn process_icon_worker_results(&mut self, ctx: &egui::Context) {
        let max_icon_uploads = if self.is_video_playing_docked() { 8 } else { 32 };
        let mut icon_uploads = 0;

        while icon_uploads < max_icon_uploads {
            if let Ok((path, icon_generation, pixels, width, height)) =
                self.icon_res_receiver.try_recv()
            {
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

        if icon_uploads >= max_icon_uploads {
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
        let mut received_any = false;

        while let Ok(msg) = self.folder_size_state.res_receiver.try_recv() {
            match msg {
                crate::app::folder_size_state::FolderSizeMessage::Progress {
                    folder_path,
                    total_size,
                } => {
                    self.folder_size_state.cache.put(folder_path, total_size);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Complete {
                    folder_path,
                    total_size,
                } => {
                    self.folder_size_state.loading.remove(&folder_path);
                    self.folder_size_state.cache.put(folder_path, total_size);
                    received_any = true;
                }
                crate::app::folder_size_state::FolderSizeMessage::Cancelled { folder_path } => {
                    self.folder_size_state.loading.remove(&folder_path);
                    self.folder_size_state.cache.pop(&folder_path);
                    received_any = true;
                }
            }
        }

        received_any
    }
}
