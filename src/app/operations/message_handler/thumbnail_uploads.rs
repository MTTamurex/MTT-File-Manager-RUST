use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_PENDING_THUMBNAILS: usize = 64;
const CRITICAL_FRAME_TIME_MS: f32 = 33.33;
const SEVERE_FRAME_TIME_MS: f32 = 25.0;

impl ImageViewerApp {
    pub(super) fn process_thumbnail_upload_pipeline(&mut self, ctx: &egui::Context) -> bool {
        let mut received_any = false;

        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            if thumbnail_data.generation != self.generation {
                continue;
            }

            self.cache_manager.finish_loading(&thumbnail_data.path);

            if thumbnail_data.image_data.is_empty() {
                self.cache_manager
                    .mark_as_failed(thumbnail_data.path.clone());

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

            while self.pending_thumbnails.len() >= MAX_PENDING_THUMBNAILS {
                if let Some(old) = self.pending_thumbnails.pop_front() {
                    self.cache_manager.finish_pending_upload(&old.path);
                }
            }

            self.cache_manager
                .start_pending_upload(thumbnail_data.path.clone());
            self.pending_thumbnails.push_back(thumbnail_data);
            received_any = true;
        }

        let is_scrolling = self.last_scroll_time.elapsed() < Duration::from_millis(100);
        let is_video_playing = self.is_video_playing_docked();
        let is_performance_critical = self.frame_time_peak_ms > CRITICAL_FRAME_TIME_MS;
        let is_performance_severe = self.frame_time_peak_ms > SEVERE_FRAME_TIME_MS;

        let base_max_uploads = if is_performance_critical {
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
                        self.pending_thumbnails.push_back(thumbnail_data);
                        deferred_count += 1;
                        if deferred_count > max_uploads_per_frame * 3 {
                            break;
                        }
                        continue;
                    }
                }

                let path = thumbnail_data.path.clone();
                let width = thumbnail_data.width;
                let height = thumbnail_data.height;
                let rgba_data = thumbnail_data.image_data;

                let texture = ctx.load_texture(
                    path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );

                self.cache_manager
                    .put_rgba_data(path.clone(), rgba_data, width, height);
                self.cache_manager
                    .put_thumbnail(path.clone(), texture.clone());
                self.cache_manager.finish_pending_upload(&path);

                if let Some(selected_file) = &self.selected_file {
                    if selected_file.path == path {
                        self.selected_thumbnail = Some(texture);
                    }
                }

                uploads_this_frame += 1;
                received_any = true;
            } else {
                break;
            }
        }

        if !self.pending_thumbnails.is_empty() {
            ctx.request_repaint();
        }

        self.process_folder_preview_uploads(ctx, is_performance_critical, is_video_playing);
        received_any
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

        let mut folder_uploads = 0;
        while folder_uploads < max_folder_uploads {
            if let Ok(data) = self.folder_preview_receiver.try_recv() {
                self.cache_manager.finish_folder_preview_loading(&data.path);

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

        if folder_uploads >= max_folder_uploads {
            ctx.request_repaint();
        }
    }
}
