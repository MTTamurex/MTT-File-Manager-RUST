use crate::image_viewer::cache::{LoadPriority, PrefetchEngine, WindowCache};
use crate::image_viewer::indexer::{self, ImageSequence};
use crate::image_viewer::loader;
use eframe::egui;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

mod filmstrip;
mod gif_export;
mod rendering;

use filmstrip::FilmstripState;
use gif_export::{GifAnimation, ViewerStatusMessage};

const DEFAULT_CACHE_RADIUS: usize = 6;
const MIN_ZOOM_FACTOR: f32 = 0.10;
const MAX_ZOOM_FACTOR: f32 = 8.0;
/// Minimum interval between navigation actions to prevent flooding workers
/// during rapid key-repeat. 20 ms ≈ 50 navigations/sec — fast enough to feel
/// responsive but slow enough for workers to keep up.
const MIN_NAVIGATE_INTERVAL: Duration = Duration::from_millis(20);

pub struct DedicatedImageViewerApp {
    pub(super) sequence: ImageSequence,
    pub(super) current_index: usize,
    pub(super) worker_count: usize,
    pub(super) cache: WindowCache,
    pub(super) prefetch: PrefetchEngine,
    pub(super) external_open_rx: std::sync::mpsc::Receiver<std::path::PathBuf>,
    pub(super) requested_jobs: HashSet<usize>,
    pub(super) texture_serial: u64,
    pub(super) texture: Option<egui::TextureHandle>,
    pub(super) last_error: Option<String>,
    pub(super) zoom_factor: f32,
    pub(super) zoom_percent_display: f32,
    pub(super) image_resolution: Option<(u32, u32)>,
    pub(super) repaint_ctx_set: bool,
    /// Animated GIF state; `Some` when the current image is a multi-frame GIF.
    pub(super) gif_animation: Option<GifAnimation>,
    /// Index for which GIF loading was already attempted (avoids retrying).
    pub(super) gif_loaded_index: Option<usize>,
    /// In-flight async GIF decode: (target_index, receiver).
    /// `None` when idle; dropped automatically if user navigates away.
    pub(super) gif_decode_rx: Option<(
        usize,
        std::sync::mpsc::Receiver<Result<Vec<loader::GifAnimationFrame>, String>>,
    )>,
    pub(super) conversion_rx: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    pub(super) conversion_in_progress: bool,
    pub(super) status_message: Option<ViewerStatusMessage>,
    /// Timestamp of the last navigation action (for key-repeat throttling).
    pub(super) last_navigate_instant: std::time::Instant,
    pub(super) filmstrip: FilmstripState,
    /// Whether to apply dark theme on first frame.
    pub(super) dark_mode: bool,
}

impl DedicatedImageViewerApp {
    pub fn new(
        sequence: ImageSequence,
        external_open_rx: std::sync::mpsc::Receiver<std::path::PathBuf>,
        dark_mode: bool,
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
            conversion_rx: None,
            conversion_in_progress: false,
            status_message: None,
            last_navigate_instant: std::time::Instant::now(),
            filmstrip: FilmstripState::new(),
            dark_mode,
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
        self.conversion_rx = None;
        self.conversion_in_progress = false;
        self.status_message = None;
        self.last_navigate_instant = std::time::Instant::now();
        self.filmstrip.reset();

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

    pub(super) fn current_path(&self) -> Option<&std::path::PathBuf> {
        self.sequence.entries.get(self.current_index)
    }

    pub(super) fn current_filename(&self) -> String {
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

    pub(super) fn navigate_to(&mut self, index: usize, ctx: &egui::Context) {
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

        self.filmstrip.scroll_to_current = true;
    }

    pub(super) fn navigate_prev(&mut self, ctx: &egui::Context) {
        if self.current_index == 0 {
            return;
        }
        self.navigate_to(self.current_index - 1, ctx);
    }

    pub(super) fn navigate_next(&mut self, ctx: &egui::Context) {
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
}

impl eframe::App for DedicatedImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if !self.repaint_ctx_set {
            self.prefetch.set_repaint_ctx(ctx.clone());
            self.repaint_ctx_set = true;

            // Apply theme on first frame (cc.egui_ctx.set_visuals in creator
            // can be overridden by the platform integration).
            if self.dark_mode {
                ctx.set_visuals(egui::Visuals::dark());
            } else {
                ctx.set_visuals(egui::Visuals::light());
            }

            // Apply dark/light title bar on the native Windows decoration.
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(wh) = handle.as_raw() {
                    let hwnd = windows::Win32::Foundation::HWND(wh.hwnd.get() as _);
                    crate::infrastructure::windows::window_corners::apply_dark_title_bar(
                        hwnd,
                        self.dark_mode,
                    );
                }
            }
        }

        self.handle_external_open_requests(ctx);
        self.handle_shortcuts(ctx);
        self.sync_window_title(ctx);

        self.handle_prefetch_results(ctx);
        self.cache.evict_over_budget(self.current_index);

        if self.sequence.entries.is_empty() {
            self.render_top_bar(ctx);
            self.render_bottom_bar(ctx);
            self.render_center(ctx);
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
        self.poll_conversion();

        self.poll_filmstrip_results(ctx);
        self.evict_filmstrip_textures();

        self.render_top_bar(ctx);
        self.render_bottom_bar(ctx);
        self.render_filmstrip(ctx);
        self.render_center(ctx);

        // Low-frequency fallback poll — workers trigger immediate repaints via
        // ctx.request_repaint(), but this ensures progress even if the signal
        // is missed (e.g. during the first frame before ctx is propagated).
        if !self.cache.has(self.current_index) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }
}

