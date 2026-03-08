use crate::image_viewer::cache::{LoadPriority, PrefetchEngine, WindowCache};
use crate::image_viewer::indexer::{self, ImageSequence};
use crate::image_viewer::loader;
use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;
use rust_i18n::t;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CACHE_RADIUS: usize = 6;
const MIN_ZOOM_FACTOR: f32 = 0.10;
const MAX_ZOOM_FACTOR: f32 = 8.0;
/// Minimum interval between navigation actions to prevent flooding workers
/// during rapid key-repeat. 20 ms ≈ 50 navigations/sec — fast enough to feel
/// responsive but slow enough for workers to keep up.
const MIN_NAVIGATE_INTERVAL: Duration = Duration::from_millis(20);

/// Holds pre-uploaded textures for each frame of an animated GIF along with
/// display timing information.
struct GifAnimation {
    textures: Vec<egui::TextureHandle>,
    delays_ms: Vec<u32>,
    current_frame: usize,
    frame_started: std::time::Instant,
}

pub struct DedicatedImageViewerApp {
    sequence: ImageSequence,
    current_index: usize,
    worker_count: usize,
    cache: WindowCache,
    prefetch: PrefetchEngine,
    external_open_rx: std::sync::mpsc::Receiver<std::path::PathBuf>,
    requested_jobs: HashSet<usize>,
    texture_serial: u64,
    texture: Option<egui::TextureHandle>,
    last_error: Option<String>,
    zoom_factor: f32,
    zoom_percent_display: f32,
    image_resolution: Option<(u32, u32)>,
    repaint_ctx_set: bool,
    /// Animated GIF state; `Some` when the current image is a multi-frame GIF.
    gif_animation: Option<GifAnimation>,
    /// Index for which GIF loading was already attempted (avoids retrying).
    gif_loaded_index: Option<usize>,
    /// In-flight async GIF decode: (target_index, receiver).
    /// `None` when idle; dropped automatically if user navigates away.
    gif_decode_rx: Option<(
        usize,
        std::sync::mpsc::Receiver<Result<Vec<loader::GifAnimationFrame>, String>>,
    )>,
    /// Timestamp of the last navigation action (for key-repeat throttling).
    last_navigate_instant: std::time::Instant,
}

impl DedicatedImageViewerApp {
    pub fn new(
        sequence: ImageSequence,
        external_open_rx: std::sync::mpsc::Receiver<std::path::PathBuf>,
    ) -> Self {
        let worker_count = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(2)
            .clamp(1, 4);

        let start_index = sequence.current_index.min(sequence.entries.len().saturating_sub(1));
        let cache = Self::build_initial_cache(&sequence, start_index);

        let app = Self {
            current_index: start_index,
            sequence,
            worker_count,
            cache,
            prefetch: PrefetchEngine::new(worker_count, DEFAULT_CACHE_RADIUS),
            external_open_rx,
            requested_jobs: HashSet::new(),
            texture_serial: 0,
            texture: None,
            last_error: None,
            zoom_factor: 1.0,
            zoom_percent_display: 100.0,
            image_resolution: None,
            repaint_ctx_set: false,
            gif_animation: None,
            gif_loaded_index: None,
            gif_decode_rx: None,
            last_navigate_instant: std::time::Instant::now(),
        };

        app.prefetch.set_center(start_index);
        app
    }

    fn build_initial_cache(sequence: &ImageSequence, index: usize) -> WindowCache {
        let mut cache = WindowCache::new(DEFAULT_CACHE_RADIUS);
        if let Some(path) = sequence.entries.get(index) {
            match loader::decode_full_frame_with_priority(path, loader::DecodePriority::Interactive)
            {
                Ok(frame) => {
                    cache.put(index, Arc::new(frame));
                }
                Err(err) => {
                    log::warn!("[IMAGE-VIEWER] failed to sync-load image: {}", err);
                }
            }
        }
        cache
    }

    fn open_requested_path(&mut self, path: std::path::PathBuf, ctx: &egui::Context) {
        let sequence = match indexer::build_sequence(&path) {
            Ok(sequence) => sequence,
            Err(err) => {
                log::warn!(
                    "[IMAGE-VIEWER] failed to build sequence for '{}': {}",
                    path.display(),
                    err
                );
                ImageSequence::single(path.clone())
            }
        };

        let start_index = sequence.current_index.min(sequence.entries.len().saturating_sub(1));
        let cache = Self::build_initial_cache(&sequence, start_index);

        self.sequence = sequence;
        self.current_index = start_index;
        self.cache = cache;
        self.prefetch = PrefetchEngine::new(self.worker_count, DEFAULT_CACHE_RADIUS);
        if self.repaint_ctx_set {
            self.prefetch.set_repaint_ctx(ctx.clone());
        }
        self.prefetch.set_center(start_index);
        self.requested_jobs.clear();
        self.texture = None;
        self.last_error = None;
        self.zoom_factor = 1.0;
        self.zoom_percent_display = 100.0;
        self.image_resolution = None;
        self.gif_animation = None;
        self.gif_loaded_index = None;
        self.gif_decode_rx = None;
        self.last_navigate_instant = std::time::Instant::now();

        self.try_show_cached_current(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn handle_external_open_requests(&mut self, ctx: &egui::Context) {
        let mut latest_path = None;

        loop {
            match self.external_open_rx.try_recv() {
                Ok(path) => latest_path = Some(path),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        if let Some(path) = latest_path {
            self.open_requested_path(path, ctx);
        }
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

    fn request_job_if_needed(&mut self, index: usize, priority: LoadPriority) {
        if self.cache.has(index) {
            return;
        }

        if priority != LoadPriority::Urgent && self.requested_jobs.contains(&index) {
            return;
        }

        let Some(path) = self.sequence.entries.get(index).cloned() else {
            return;
        };

        if self
            .prefetch
            .request(index, path, priority)
        {
            if priority != LoadPriority::Urgent {
                self.requested_jobs.insert(index);
            }
        }
    }

    fn schedule_window_requests(&mut self) {
        if self.sequence.entries.is_empty() {
            return;
        }

        let center = self.current_index;
        let total = self.sequence.entries.len();
        let min_idx = center.saturating_sub(self.cache.radius());
        let max_idx = (center + self.cache.radius()).min(total - 1);

        // Current image: highest priority
        self.request_job_if_needed(center, LoadPriority::Urgent);

        // Immediate neighbors: high priority
        let left = center.saturating_sub(1);
        if left != center {
            self.request_job_if_needed(left, LoadPriority::High);
        }

        let right = (center + 1).min(total - 1);
        if right != center {
            self.request_job_if_needed(right, LoadPriority::High);
        }

        // Rest of window: normal priority
        for idx in min_idx..=max_idx {
            if idx == center || idx == left || idx == right {
                continue;
            }
            self.request_job_if_needed(idx, LoadPriority::Normal);
        }
    }

    fn upload_frame(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        frame: Arc<loader::DecodedFrame>,
    ) {
        if frame.width == 0 || frame.height == 0 || frame.rgba.is_empty() {
            self.last_error = Some("decoded frame is empty".to_string());
            return;
        }

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.rgba,
        );

        let texture_name = format!(
            "iv-{}-{}",
            index, self.texture_serial
        );
        self.texture_serial = self.texture_serial.wrapping_add(1);

        let texture = ctx.load_texture(texture_name, color_image, egui::TextureOptions::LINEAR);

        self.texture = Some(texture);
        self.image_resolution = Some((frame.width, frame.height));
        self.last_error = None;
    }

    fn try_show_cached_current(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.cache.get(self.current_index) else {
            // Do NOT clear texture here — keep showing the previous image
            // until the new one arrives (matches viewskater behavior).
            return;
        };

        self.upload_frame(ctx, self.current_index, frame);
    }

    fn handle_prefetch_results(&mut self, ctx: &egui::Context) {
        for output in self.prefetch.drain_results(256) {
            self.requested_jobs.remove(&output.index);

            match output.frame {
                Ok(frame) => {
                    let frame = Arc::new(frame);
                    self.cache.put(output.index, Arc::clone(&frame));

                    if output.index == self.current_index {
                        self.upload_frame(ctx, output.index, frame);
                    }
                }
                Err(err) => {
                    // Interrupted = job was skipped by worker (too far from center).
                    // Just remove from requested_jobs so it can be retried later.
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    if output.index == self.current_index {
                        self.last_error = Some(format!("{}", err));
                    }
                }
            }
        }
    }

    // --- GIF animation helpers ---

    fn is_current_gif(&self) -> bool {
        self.current_path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("gif"))
            .unwrap_or(false)
    }

    /// Kicks off an async GIF decode if the current image is a GIF that hasn't
    /// been loaded yet. Returns immediately — frames arrive via `poll_gif_decode`.
    fn load_gif_if_needed(&mut self, ctx: &egui::Context) {
        if self.gif_loaded_index == Some(self.current_index) {
            return;
        }
        // Mark as attempted to avoid re-spawning on every frame.
        self.gif_loaded_index = Some(self.current_index);
        self.gif_animation = None;
        self.gif_decode_rx = None; // drop any stale decode

        if !self.is_current_gif() {
            return;
        }

        let Some(path) = self.current_path().cloned() else {
            return;
        };

        let (tx, rx) = std::sync::mpsc::channel();
        let ctx_clone = ctx.clone();
        std::thread::Builder::new()
            .name("gif-decode".into())
            .spawn(move || {
                let result = loader::decode_gif_frames(&path)
                    .map_err(|e| e.to_string());
                let _ = tx.send(result);
                ctx_clone.request_repaint();
            })
            .ok();
        self.gif_decode_rx = Some((self.current_index, rx));
    }

    /// Polls the in-flight GIF decode channel. When decoding is complete,
    /// uploads all frames as textures and builds the `GifAnimation` struct.
    fn poll_gif_decode(&mut self, ctx: &egui::Context) {
        let Some((decode_index, rx)) = &self.gif_decode_rx else {
            return;
        };
        let decode_index = *decode_index;

        // User navigated away before the decode finished — discard.
        if decode_index != self.current_index {
            self.gif_decode_rx = None;
            return;
        }

        match rx.try_recv() {
            Ok(Ok(frames)) if frames.len() > 1 => {
                let (w, h) = (frames[0].frame.width, frames[0].frame.height);
                let mut textures = Vec::with_capacity(frames.len());
                let mut delays = Vec::with_capacity(frames.len());

                for (i, gif_frame) in frames.into_iter().enumerate() {
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [gif_frame.frame.width as usize, gif_frame.frame.height as usize],
                        &gif_frame.frame.rgba,
                    );
                    let tex = ctx.load_texture(
                        format!("iv-gif-{}-{}", decode_index, i),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    textures.push(tex);
                    delays.push(gif_frame.delay_ms);
                }

                self.gif_animation = Some(GifAnimation {
                    textures,
                    delays_ms: delays,
                    current_frame: 0,
                    frame_started: std::time::Instant::now(),
                });
                self.image_resolution = Some((w, h));
                self.gif_decode_rx = None;
            }
            Ok(Ok(_)) => {
                // Single-frame GIF — static path renders it.
                self.gif_decode_rx = None;
            }
            Ok(Err(e)) => {
                log::warn!("[IMAGE-VIEWER] GIF decode failed for index {}: {}", decode_index, e);
                self.gif_decode_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Decode still running — ctx.request_repaint was called by the worker.
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.gif_decode_rx = None;
            }
        }
    }

    /// Advances the animation to the next frame when the current frame's delay
    /// has elapsed, and schedules a repaint for the next frame transition.
    fn advance_gif_frame(&mut self, ctx: &egui::Context) {
        let Some(anim) = self.gif_animation.as_mut() else {
            return;
        };

        let delay = Duration::from_millis(anim.delays_ms[anim.current_frame] as u64);
        if anim.frame_started.elapsed() >= delay {
            anim.current_frame = (anim.current_frame + 1) % anim.textures.len();
            anim.frame_started = std::time::Instant::now();
        }

        let elapsed = anim.frame_started.elapsed();
        let remaining = delay.saturating_sub(elapsed).max(Duration::from_millis(10));
        ctx.request_repaint_after(remaining);
    }

    fn navigate_to(&mut self, index: usize, ctx: &egui::Context) {
        if index >= self.sequence.entries.len() {
            return;
        }

        if self.current_index == index {
            return;
        }

        // Throttle rapid navigations (e.g. holding arrow key) so workers
        // can keep up with decode requests.
        if self.last_navigate_instant.elapsed() < MIN_NAVIGATE_INTERVAL {
            return;
        }
        self.last_navigate_instant = std::time::Instant::now();

        let old_index = self.current_index;
        self.current_index = index;
        self.zoom_factor = 1.0;

        // Reset GIF animation state for the new image.
        self.gif_animation = None;
        self.gif_loaded_index = None;

        // Update the atomic center so workers skip irrelevant jobs.
        self.prefetch.set_center(index);

        // Evict cache entries outside the sliding window.
        let total = self.sequence.entries.len();
        self.cache.retain_window(index, total);

        // Prune requested_jobs to the current window only (don't clear —
        // clearing causes mass re-submission of duplicate jobs).
        let radius = self.cache.radius();
        let min_idx = index.saturating_sub(radius);
        let max_idx = (index + radius).min(total.saturating_sub(1));
        self.requested_jobs
            .retain(|&idx| idx >= min_idx && idx <= max_idx);

        // Show from cache immediately if available; keep old image otherwise
        // (like viewskater: don't clear texture, don't show spinner).
        self.try_show_cached_current(ctx);

        // Like viewskater: only request the NEW tail image that entered the
        // window, plus the current image if not cached. All other images
        // should already be cached or in-flight from previous steps.
        let tail = if index > old_index {
            // Moving right → new right edge
            (index + radius).min(total.saturating_sub(1))
        } else {
            // Moving left → new left edge
            index.saturating_sub(radius)
        };
        self.request_job_if_needed(index, LoadPriority::Urgent);
        if tail != index {
            self.request_job_if_needed(tail, LoadPriority::High);
        }
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
                    .add_enabled(prev_enabled, egui::Button::new(&*t!("imageviewer.previous")))
                    .clicked()
                {
                    self.navigate_prev(ctx);
                }

                if ui
                    .add_enabled(next_enabled, egui::Button::new(&*t!("imageviewer.next")))
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
                ui.label(&*t!("imageviewer.zoom"));

                let mut slider_zoom = self.zoom_factor;
                let slider = egui::Slider::new(&mut slider_zoom, MIN_ZOOM_FACTOR..=MAX_ZOOM_FACTOR)
                    .show_value(false);

                if ui.add_sized([260.0, 20.0], slider).changed() {
                    self.zoom_factor = slider_zoom.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                }

                ui.label(format!("{:.0}%", self.zoom_percent_display.round()));

                ui.separator();
                if let Some((w, h)) = self.image_resolution {
                    ui.label(&*t!("imageviewer.resolution", w = w, h = h));
                } else {
                    ui.label(&*t!("imageviewer.resolution_none"));
                }
            });
        });
    }

    fn render_center(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Prefer the current GIF animation frame; fall back to static texture.
            // Clone is cheap: egui::TextureHandle is reference-counted.
            let active_tex: Option<egui::TextureHandle> = if let Some(anim) = &self.gif_animation {
                anim.textures.get(anim.current_frame).cloned()
            } else {
                self.texture.clone()
            };

            if let Some(tex) = active_tex {
                // egui layout works in points, while texture size is in pixels.
                // Convert first to avoid implicit upscale on high-DPI monitors.
                let pixels_per_point = ui.ctx().pixels_per_point().max(f32::EPSILON);
                let tex_size = tex.size_vec2() / pixels_per_point;
                let viewport_size = ui.available_size();

                // Fit-to-window only downscales; never upscales small images.
                let fit_scale = if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
                    1.0
                } else {
                    let sx = viewport_size.x / tex_size.x;
                    let sy = viewport_size.y / tex_size.y;
                    sx.min(sy).min(1.0)
                };

                let draw_size = tex_size * fit_scale * self.zoom_factor;
                self.zoom_percent_display = fit_scale * self.zoom_factor * 100.0;

                let available_rect = ui.available_rect_before_wrap();
                let horizontal_scroll_bar_rect = egui::Rect::from_min_max(
                    egui::pos2(available_rect.left(), available_rect.bottom()),
                    egui::pos2(available_rect.right(), available_rect.bottom()),
                );

                egui::ScrollArea::both()
                    .id_salt("image_viewer_center_scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                    .scroll_bar_rect(horizontal_scroll_bar_rect)
                    .show(ui, |ui| {
                        let canvas_size = egui::vec2(
                            draw_size.x.max(viewport_size.x),
                            draw_size.y.max(viewport_size.y),
                        );
                        // Ensure content size is allowed to exceed viewport on both axes,
                        // so horizontal and vertical scrollbars can appear when needed.
                        ui.set_min_size(canvas_size);
                        let (canvas_rect, _) = ui.allocate_at_least(canvas_size, egui::Sense::hover());

                        let image = egui::Image::new(&tex)
                            .fit_to_exact_size(draw_size)
                            .sense(egui::Sense::click());
                        let image_rect = egui::Rect::from_center_size(canvas_rect.center(), draw_size);
                        let response = ui
                            .put(image_rect, image);

                        if response.hovered() {

                            let wheel_delta = ui.input(|i| i.raw_scroll_delta.y);
                            if wheel_delta.abs() > f32::EPSILON {
                                // Granular zoom: track wheel delta continuously.
                                let factor = 1.0 + wheel_delta * 0.0015;
                                if factor > 0.0 {
                                    self.zoom_factor = (self.zoom_factor * factor)
                                        .clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                                }
                            }

                            let left_click =
                                ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
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
                    });
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
                        let available = ui.available_size();
                        ui.allocate_response(available, egui::Sense::hover());
                    },
                );
            }
        });
    }
}

impl eframe::App for DedicatedImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.repaint_ctx_set {
            self.prefetch.set_repaint_ctx(ctx.clone());
            self.repaint_ctx_set = true;
        }

        self.handle_external_open_requests(ctx);
        self.handle_shortcuts(ctx);
        self.sync_window_title(ctx);

        self.handle_prefetch_results(ctx);
        self.cache.evict_over_budget(self.current_index);

        if self.sequence.entries.is_empty() {
            self.render_top_bar(ctx);
            self.render_center(ctx);
            self.render_bottom_bar(ctx);
            return;
        }

        if self.texture.is_none() {
            self.try_show_cached_current(ctx);
        }

        // Fill any gaps in the cache window only when the user is not rapidly
        // navigating. During rapid navigation navigate_to() already requests
        // the urgent current image + the new tail; flooding workers with the
        // full window would waste time decoding images we'll skip past.
        let navigating_fast =
            self.last_navigate_instant.elapsed() < Duration::from_millis(80);
        if !navigating_fast {
            self.schedule_window_requests();
        }

        // Decode and upload all GIF frames (once per image), then advance timer.
        self.load_gif_if_needed(ctx);
        self.poll_gif_decode(ctx);
        self.advance_gif_frame(ctx);

        self.render_top_bar(ctx);
        self.render_center(ctx);
        self.render_bottom_bar(ctx);

        // Low-frequency fallback poll — workers trigger immediate repaints via
        // ctx.request_repaint(), but this ensures progress even if the signal
        // is missed (e.g. during the first frame before ctx is propagated).
        if !self.cache.has(self.current_index) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}

