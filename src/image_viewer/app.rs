use crate::image_viewer::cache::{LoadKind, LoadPriority, PrefetchEngine, WindowCache};
use crate::image_viewer::indexer::ImageSequence;
use crate::image_viewer::loader;
use crate::image_viewer::metrics::Metrics;
use eframe::egui;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CACHE_RADIUS: usize = 6;
const MIN_ZOOM_FACTOR: f32 = 0.10;
const MAX_ZOOM_FACTOR: f32 = 8.0;

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
    zoom_factor: f32,
    zoom_percent_display: f32,
    image_resolution: Option<(u32, u32)>,
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
            zoom_factor: 1.0,
            zoom_percent_display: 100.0,
            image_resolution: None,
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
        if matches!(kind, LoadKind::Full) || self.image_resolution.is_none() {
            self.image_resolution = Some((frame.width, frame.height));
        }
        self.last_error = None;
    }

    fn try_show_cached_current(&mut self, ctx: &egui::Context) {
        let Some((kind, frame)) = self.cache.get_best(self.current_index) else {
            self.texture = None;
            self.texture_kind = None;
            self.image_resolution = None;
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
        self.zoom_factor = 1.0;
        self.image_resolution = None;
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
                    .add_enabled(prev_enabled, egui::Button::new("◀ Anterior"))
                    .clicked()
                {
                    self.navigate_prev(ctx);
                }

                if ui
                    .add_enabled(next_enabled, egui::Button::new("Próximo ▶"))
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

    fn render_bottom_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("image_viewer_bottom_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Zoom");

                let mut slider_zoom = self.zoom_factor;
                let slider = egui::Slider::new(&mut slider_zoom, MIN_ZOOM_FACTOR..=MAX_ZOOM_FACTOR)
                    .show_value(false);

                if ui.add_sized([260.0, 20.0], slider).changed() {
                    self.zoom_factor = slider_zoom.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                }

                ui.label(format!("{:.0}%", self.zoom_percent_display.round()));

                ui.separator();
                if let Some((w, h)) = self.image_resolution {
                    ui.label(format!("Resolução: {} × {}", w, h));
                } else {
                    ui.label("Resolução: —");
                }
            });
        });
    }

    fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tex) = &self.texture {
                // egui layout works in points, while texture size is in pixels.
                // Convert first to avoid implicit upscale on high-DPI monitors.
                let pixels_per_point = ui.ctx().pixels_per_point().max(f32::EPSILON);
                let tex_size = tex.size_vec2() / pixels_per_point;
                let panel_rect = ui.max_rect();
                let avail = panel_rect.size();

                // Fit-to-window only downscales; never upscales small images.
                let fit_scale = if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
                    1.0
                } else {
                    let sx = avail.x / tex_size.x;
                    let sy = avail.y / tex_size.y;
                    sx.min(sy).min(1.0)
                };

                let draw_size = tex_size * fit_scale * self.zoom_factor;
                self.zoom_percent_display = fit_scale * self.zoom_factor * 100.0;

                let image = egui::Image::new(tex)
                    .fit_to_exact_size(draw_size)
                    .sense(egui::Sense::click());
                let image_rect = egui::Rect::from_center_size(panel_rect.center(), draw_size);
                let response = ui.put(image_rect, image).on_hover_cursor(egui::CursorIcon::ZoomIn);

                if response.hovered() {
                    // Native ZoomIn cursor is not consistently available on Windows/winit.
                    // Hide the system cursor and render an explicit magnifier overlay.
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);

                    if let Some(pointer_pos) = ui
                        .ctx()
                        .pointer_latest_pos()
                        .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                    {
                        let lens_center = pointer_pos + egui::vec2(10.0, 10.0);
                        let radius = 7.0;
                        let handle_start = lens_center + egui::vec2(4.0, 4.0);
                        let handle_end = handle_start + egui::vec2(6.0, 6.0);
                        let painter = ui.ctx().layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            egui::Id::new("image_viewer_zoom_cursor"),
                        ));

                        let shadow = egui::Color32::from_black_alpha(180);
                        painter.circle_stroke(
                            lens_center,
                            radius,
                            egui::Stroke::new(3.0, shadow),
                        );
                        painter.line_segment(
                            [handle_start, handle_end],
                            egui::Stroke::new(3.0, shadow),
                        );

                        painter.circle_stroke(
                            lens_center,
                            radius,
                            egui::Stroke::new(1.6, egui::Color32::WHITE),
                        );
                        painter.line_segment(
                            [handle_start, handle_end],
                            egui::Stroke::new(1.6, egui::Color32::WHITE),
                        );
                    }

                    let wheel_delta = ui.input(|i| i.raw_scroll_delta.y);
                    if wheel_delta.abs() > f32::EPSILON {
                        // Granular zoom: track wheel delta continuously.
                        let factor = 1.0 + wheel_delta * 0.0015;
                        if factor > 0.0 {
                            self.zoom_factor =
                                (self.zoom_factor * factor).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                        }
                    }

                    let left_click = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
                    if left_click {
                        self.zoom_factor =
                            (self.zoom_factor * 1.25).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                    }

                    let right_click =
                        ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
                    if right_click {
                        self.zoom_factor =
                            (self.zoom_factor / 1.25).clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                    }
                }
            } else if let Some(err) = &self.last_error {
                self.zoom_percent_display = 100.0;
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(220, 80, 80)));
                    },
                );
            } else if self.sequence.entries.is_empty() {
                self.zoom_percent_display = 100.0;
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.label("No image available");
                    },
                );
            } else {
                self.zoom_percent_display = 100.0;
                ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::TopDown),
                    |ui| {
                        ui.add(egui::Spinner::new());
                        ui.label("Loading image...");
                    },
                );
            }
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

