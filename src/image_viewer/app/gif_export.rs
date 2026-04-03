use crate::image_viewer::loader;
use eframe::egui;
use rfd::FileDialog;
use rust_i18n::t;
use std::sync::Arc;
use std::time::Duration;

/// Holds pre-uploaded textures for each frame of an animated GIF along with
/// display timing information.
pub(in crate::image_viewer) struct GifAnimation {
    pub(super) frames: Vec<Arc<loader::DecodedFrame>>,
    pub(super) textures: Vec<egui::TextureHandle>,
    pub(super) delays_ms: Vec<u32>,
    pub(super) current_frame: usize,
    pub(super) frame_started: std::time::Instant,
}

pub(in crate::image_viewer) struct ViewerStatusMessage {
    pub(super) text: String,
    pub(super) is_error: bool,
}

impl super::DedicatedImageViewerApp {
    // --- Export helpers ---

    pub(super) fn export_format_label(format: loader::ExportImageFormat) -> String {
        match format {
            loader::ExportImageFormat::Png => t!("imageviewer.format_png").to_string(),
            loader::ExportImageFormat::Jpeg => t!("imageviewer.format_jpeg").to_string(),
            loader::ExportImageFormat::WebP => t!("imageviewer.format_webp").to_string(),
            loader::ExportImageFormat::Bmp => t!("imageviewer.format_bmp").to_string(),
            loader::ExportImageFormat::Tiff => t!("imageviewer.format_tiff").to_string(),
        }
    }

    fn suggested_export_filename(&self, format: loader::ExportImageFormat) -> String {
        let stem = self
            .current_path()
            .and_then(|path| path.file_stem())
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "image".to_string());

        format!("{}.{}", stem, format.extension())
    }

    fn pick_export_path(
        &self,
        format: loader::ExportImageFormat,
    ) -> Option<std::path::PathBuf> {
        let mut dialog = FileDialog::new()
            .add_filter(format.filter_label(), &[format.extension()])
            .set_file_name(&self.suggested_export_filename(format));

        if let Some(current_path) = self.current_path() {
            if let Some(parent) = current_path.parent() {
                dialog = dialog.set_directory(parent);
            }
        }

        dialog
            .save_file()
            .map(|path| loader::normalize_export_path(&path, format))
    }

    fn current_export_frame(&self) -> Result<loader::DecodedFrame, String> {
        if let Some(anim) = &self.gif_animation {
            if let Some(frame) = anim.frames.get(anim.current_frame) {
                return Ok((**frame).clone());
            }
        }

        if let Some(frame) = self.cache.get(self.current_index) {
            return Ok(frame.as_ref().clone());
        }

        let path = self
            .current_path()
            .ok_or_else(|| t!("imageviewer.convert_no_image").to_string())?;
        loader::decode_full_frame(path).map_err(|err| err.to_string())
    }

    pub(super) fn start_conversion(&mut self, format: loader::ExportImageFormat, ctx: &egui::Context) {
        if self.conversion_in_progress {
            return;
        }

        let Some(output_path) = self.pick_export_path(format) else {
            return;
        };

        let frame = match self.current_export_frame() {
            Ok(frame) => frame,
            Err(err) => {
                self.status_message = Some(ViewerStatusMessage {
                    text: t!("imageviewer.convert_error", error = err).to_string(),
                    is_error: true,
                });
                return;
            }
        };

        let (tx, rx) = std::sync::mpsc::channel();
        let repaint_ctx = ctx.clone();
        let worker_path = output_path.clone();
        let format_label = Self::export_format_label(format);

        self.conversion_rx = Some(rx);
        self.conversion_in_progress = true;
        self.status_message = Some(ViewerStatusMessage {
            text: t!("imageviewer.convert_in_progress", format = format_label).to_string(),
            is_error: false,
        });

        let spawn_result = std::thread::Builder::new()
            .name("image-convert".into())
            .spawn(move || {
                let result = loader::encode_frame_to_path(frame, format, &worker_path)
                    .map(|_| worker_path)
                    .map_err(|err| err.to_string());
                let _ = tx.send(result);
                repaint_ctx.request_repaint();
            });

        if let Err(err) = spawn_result {
            self.conversion_rx = None;
            self.conversion_in_progress = false;
            self.status_message = Some(ViewerStatusMessage {
                text: t!("imageviewer.convert_error", error = err.to_string()).to_string(),
                is_error: true,
            });
        }
    }

    pub(super) fn poll_conversion(&mut self) {
        let Some(rx) = &self.conversion_rx else {
            return;
        };

        match rx.try_recv() {
            Ok(Ok(path)) => {
                self.conversion_rx = None;
                self.conversion_in_progress = false;
                let name = path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string());
                self.status_message = Some(ViewerStatusMessage {
                    text: t!("imageviewer.convert_success", name = name).to_string(),
                    is_error: false,
                });
            }
            Ok(Err(err)) => {
                self.conversion_rx = None;
                self.conversion_in_progress = false;
                self.status_message = Some(ViewerStatusMessage {
                    text: t!("imageviewer.convert_error", error = err).to_string(),
                    is_error: true,
                });
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.conversion_rx = None;
                self.conversion_in_progress = false;
                self.status_message = Some(ViewerStatusMessage {
                    text: t!("imageviewer.convert_error", error = "worker disconnected").to_string(),
                    is_error: true,
                });
            }
        }
    }

    // --- GIF animation helpers ---

    pub(super) fn is_current_gif(&self) -> bool {
        self.current_path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("gif"))
            .unwrap_or(false)
    }

    /// Kicks off an async GIF decode if the current image is a GIF that hasn't
    /// been loaded yet. Returns immediately — frames arrive via `poll_gif_decode`.
    pub(super) fn load_gif_if_needed(&mut self, ctx: &egui::Context) {
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
    pub(super) fn poll_gif_decode(&mut self, ctx: &egui::Context) {
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
                let mut raw_frames = Vec::with_capacity(frames.len());
                let mut textures = Vec::with_capacity(frames.len());
                let mut delays = Vec::with_capacity(frames.len());

                for (i, gif_frame) in frames.into_iter().enumerate() {
                    let raw_frame = Arc::new(gif_frame.frame);
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [raw_frame.width as usize, raw_frame.height as usize],
                        &raw_frame.rgba,
                    );
                    let tex = ctx.load_texture(
                        format!("iv-gif-{}-{}", decode_index, i),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    raw_frames.push(raw_frame);
                    textures.push(tex);
                    delays.push(gif_frame.delay_ms);
                }

                self.gif_animation = Some(GifAnimation {
                    frames: raw_frames,
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
    pub(super) fn advance_gif_frame(&mut self, ctx: &egui::Context) {
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
}
