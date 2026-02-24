use crate::image_viewer::cache::{LoadKind, LoadPriority, PrefetchEngine, WindowCache};
use crate::image_viewer::indexer::ImageSequence;
use crate::image_viewer::loader;
use crate::image_viewer::metrics::Metrics;
use eframe::egui;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CACHE_RADIUS: usize = 6;

pub struct DedicatedImageViewerApp {
    sequence: ImageSequence,
    current_index: usize,
    cache: WindowCache,
    prefetch: PrefetchEngine,
    requested_jobs: HashSet<(u64, usize, LoadKind)>,
    request_sequence: u64,
    texture_serial: u64,
    texture: Option<egui::TextureHandle>,
    texture_kind: Option<LoadKind>,
    metrics: Arc<Metrics>,
    last_error: Option<String>,
}

impl DedicatedImageViewerApp {
    pub fn new(sequence: ImageSequence) -> Self {
        let worker_count = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(2)
            .clamp(1, 4);

        let mut app = Self {
            current_index: sequence.current_index.min(sequence.entries.len().saturating_sub(1)),
            sequence,
            cache: WindowCache::new(DEFAULT_CACHE_RADIUS),
            prefetch: PrefetchEngine::new(worker_count, 64),
            requested_jobs: HashSet::new(),
            request_sequence: 0,
            texture_serial: 0,
            texture: None,
            texture_kind: None,
            metrics: Arc::new(Metrics::default()),
            last_error: None,
        };

        app.bump_sequence();
        app
    }

    fn bump_sequence(&mut self) {
        self.request_sequence = self.request_sequence.wrapping_add(1);
        self.requested_jobs.clear();
        self.prefetch.set_active_sequence(self.request_sequence);
        self.cache
            .retain_window(self.current_index, self.sequence.entries.len());
    }

    fn current_path(&self) -> Option<&std::path::PathBuf> {
        self.sequence.entries.get(self.current_index)
    }

    fn current_filename(&self) -> String {
        self.current_path()
            .and_then(|p| p.file_name())
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_else(|| "<unknown>".to_string())
    }

    fn request_job_if_needed(&mut self, index: usize, kind: LoadKind, priority: LoadPriority) {
        if self.cache.has(index, kind) {
            return;
        }

        let key = (self.request_sequence, index, kind);
        if self.requested_jobs.contains(&key) {
            return;
        }

        let Some(path) = self.sequence.entries.get(index).cloned() else {
            return;
        };

        if self
            .prefetch
            .request(self.request_sequence, index, path, kind, priority)
        {
            self.requested_jobs.insert(key);
        }
    }

    fn schedule_window_requests(&mut self) {
        if self.sequence.entries.is_empty() {
            return;
        }

        let center = self.current_index;
        let min_idx = center.saturating_sub(self.cache.radius());
        let max_idx = (center + self.cache.radius()).min(self.sequence.entries.len() - 1);

        self.request_job_if_needed(center, LoadKind::Preview, LoadPriority::High);
        self.request_job_if_needed(center, LoadKind::Full, LoadPriority::High);

        let left = center.saturating_sub(1);
        if left != center {
            self.request_job_if_needed(left, LoadKind::Preview, LoadPriority::High);
            self.request_job_if_needed(left, LoadKind::Full, LoadPriority::Normal);
        }

        let right = (center + 1).min(self.sequence.entries.len() - 1);
        if right != center {
            self.request_job_if_needed(right, LoadKind::Preview, LoadPriority::High);
            self.request_job_if_needed(right, LoadKind::Full, LoadPriority::Normal);
        }

        for idx in min_idx..=max_idx {
            if idx == center || idx == left || idx == right {
                continue;
            }

            self.request_job_if_needed(idx, LoadKind::Preview, LoadPriority::Normal);

            let is_priority_full = idx == center || idx.abs_diff(center) <= 1;
            if is_priority_full {
                self.request_job_if_needed(idx, LoadKind::Full, LoadPriority::Normal);
            }
        }
    }

    fn upload_frame(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        kind: LoadKind,
        frame: Arc<loader::DecodedFrame>,
    ) {
        let upload_start = std::time::Instant::now();

        if frame.width == 0 || frame.height == 0 || frame.rgba.is_empty() {
            self.last_error = Some("decoded frame is empty".to_string());
            return;
        }

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.rgba,
        );

        let texture_name = format!(
            "image-viewer-{}-{}-{:?}-{}",
            self.request_sequence, index, kind, self.texture_serial
        );
        self.texture_serial = self.texture_serial.wrapping_add(1);

        let texture = ctx.load_texture(texture_name, color_image, egui::TextureOptions::LINEAR);

        self.metrics
            .record_upload_us(upload_start.elapsed().as_micros() as u64);
        self.texture = Some(texture);
        self.texture_kind = Some(kind);
        self.last_error = None;
    }

    fn try_show_cached_current(&mut self, ctx: &egui::Context) {
        let Some((kind, frame)) = self.cache.get_best(self.current_index) else {
            self.texture = None;
            self.texture_kind = None;
            return;
        };

        self.upload_frame(ctx, self.current_index, kind, frame);
    }

    fn handle_prefetch_results(&mut self, ctx: &egui::Context) {
        for output in self.prefetch.drain_results(32) {
            self.requested_jobs
                .remove(&(output.sequence, output.index, output.kind));

            if output.sequence != self.request_sequence {
                continue;
            }

            match output.frame {
                Ok(frame) => {
                    self.metrics.record_decode_us(output.decode_us);
                    let frame = Arc::new(frame);
                    self.cache.put(output.index, output.kind, Arc::clone(&frame));

                    if output.index == self.current_index {
                        let should_replace = match self.texture_kind {
                            None => true,
                            Some(LoadKind::Preview) => {
                                matches!(output.kind, LoadKind::Preview | LoadKind::Full)
                            }
                            Some(LoadKind::Full) => matches!(output.kind, LoadKind::Full),
                        };

                        if should_replace {
                            self.upload_frame(ctx, output.index, output.kind, frame);
                        }
                    }
                }
                Err(err) => {
                    if output.index == self.current_index {
                        self.last_error = Some(format!("{}", err));
                    }
                }
            }
        }
    }

    fn navigate_to(&mut self, index: usize, ctx: &egui::Context) {
        if index >= self.sequence.entries.len() {
            return;
        }

        if self.current_index == index {
            return;
        }

        self.current_index = index;
        self.bump_sequence();
        self.try_show_cached_current(ctx);
        self.schedule_window_requests();
    }

    fn navigate_prev(&mut self, ctx: &egui::Context) {
        if self.current_index == 0 {
            return;
        }
        self.navigate_to(self.current_index - 1, ctx);
    }

    fn navigate_next(&mut self, ctx: &egui::Context) {
        if self.current_index + 1 >= self.sequence.entries.len() {
            return;
        }
        self.navigate_to(self.current_index + 1, ctx);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let prev = ctx.input(|i| {
            i.key_pressed(egui::Key::ArrowLeft)
                || i.key_pressed(egui::Key::A)
                || i.key_pressed(egui::Key::Backspace)
        });
        if prev {
            self.navigate_prev(ctx);
        }

        let next = ctx.input(|i| {
            i.key_pressed(egui::Key::ArrowRight)
                || i.key_pressed(egui::Key::D)
                || i.key_pressed(egui::Key::Space)
        });
        if next {
            self.navigate_next(ctx);
        }
    }

    fn render_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("image_viewer_top_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let total = self.sequence.entries.len();
                let prev_enabled = self.current_index > 0;
                let next_enabled = self.current_index + 1 < total;

                if ui
                    .add_enabled(prev_enabled, egui::Button::new("◀ Prev"))
                    .clicked()
                {
                    self.navigate_prev(ctx);
                }

                if ui
                    .add_enabled(next_enabled, egui::Button::new("Next ▶"))
                    .clicked()
                {
                    self.navigate_next(ctx);
                }

                ui.separator();
                if total == 0 {
                    ui.label("0 / 0");
                } else {
                    ui.label(format!("{} / {}", self.current_index + 1, total));
                }
                ui.separator();
                ui.label(self.current_filename());
                if let Some(path) = self.current_path() {
                    ui.small(path.to_string_lossy());
                }
            });
        });
    }

    fn sync_window_title(&self, ctx: &egui::Context) {
        let title = if self.sequence.entries.is_empty() {
            "Image Viewer".to_string()
        } else {
            format!("Image Viewer - {}", self.current_filename())
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn render_bottom_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("image_viewer_bottom_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.small("Keys: ←/→, A/D, Esc");
                ui.separator();
                ui.small(format!("decode avg: {:.2} ms", self.metrics.decode_avg_ms()));
                ui.separator();
                ui.small(format!("upload avg: {:.2} ms", self.metrics.upload_avg_ms()));
                if let Some(kind) = self.texture_kind {
                    ui.separator();
                    ui.small(match kind {
                        LoadKind::Preview => "quality: preview",
                        LoadKind::Full => "quality: full",
                    });
                }
            });
        });
    }

    fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered_justified(|ui| {
                if let Some(tex) = &self.texture {
                    ui.add(
                        egui::Image::new(tex)
                            .max_size(ui.available_size())
                            .shrink_to_fit(),
                    );
                } else if let Some(err) = &self.last_error {
                    ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(220, 80, 80)));
                } else if self.sequence.entries.is_empty() {
                    ui.label("No image available");
                } else {
                    ui.add(egui::Spinner::new());
                    ui.label("Loading image...");
                }
            });
        });
    }
}

impl eframe::App for DedicatedImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ctx);
        self.sync_window_title(ctx);

        self.cache
            .retain_window(self.current_index, self.sequence.entries.len());
        self.handle_prefetch_results(ctx);

        if self.sequence.entries.is_empty() {
            self.render_top_bar(ctx);
            self.render_center(ctx);
            self.render_bottom_bar(ctx);
            return;
        }

        if self.texture.is_none() {
            self.try_show_cached_current(ctx);
        }

        self.schedule_window_requests();

        self.render_top_bar(ctx);
        self.render_center(ctx);
        self.render_bottom_bar(ctx);

        if !self.cache.has(self.current_index, LoadKind::Full) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

