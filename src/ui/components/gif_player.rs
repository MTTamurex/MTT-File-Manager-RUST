use crate::ui::components::gif_manager::GifData;
use eframe::egui;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const GIF_MAX_UPLOADS_PER_FRAME: usize = 8;

#[derive(Clone, Copy)]
struct GifFrameInfo {
    width: u32,
    height: u32,
    original_width: u32,
    original_height: u32,
    delay_ms: u64,
}

struct GifFrameUpload {
    frame_index: usize,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Clone)]
pub struct GifPlayer {
    pub path: PathBuf,
    pub data: Arc<Mutex<GifData>>,
    pub texture: Option<egui::TextureHandle>,
    textures: Vec<Option<egui::TextureHandle>>,
    frame_info: Vec<GifFrameInfo>,
    active_texture_frame: usize,
    pub current_frame: usize,
    pub last_update: Instant,
}

impl GifPlayer {
    pub fn new(path: PathBuf, data: Arc<Mutex<GifData>>) -> Self {
        Self {
            path,
            data,
            texture: None,
            textures: Vec::new(),
            frame_info: Vec::new(),
            active_texture_frame: 0,
            current_frame: 0,
            last_update: Instant::now(),
        }
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        // Use try_lock() to avoid blocking the UI thread if the decode worker
        // is pushing frames while I/O is slow or cloud-backed.
        let Some(mut data) = self.data.try_lock() else {
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        };

        data.last_used = Instant::now();

        let frame_count = data.frames.len();

        if frame_count == 0 {
            let is_complete = data.is_complete;
            drop(data);
            if !is_complete {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            return;
        }

        let is_complete = data.is_complete;

        self.textures.resize_with(frame_count, || None);

        if self.frame_info.len() < frame_count {
            for frame in data.frames.iter().skip(self.frame_info.len()) {
                self.frame_info.push(GifFrameInfo {
                    width: frame.width,
                    height: frame.height,
                    original_width: frame.original_width,
                    original_height: frame.original_height,
                    delay_ms: frame.delay_ms,
                });
            }
        }

        if self.current_frame >= frame_count {
            self.current_frame %= frame_count;
        }

        let mut uploads = Vec::new();
        let current_idx = self.current_frame;
        if self.textures[current_idx].is_none() {
            if let Some(rgba) = data.take_frame_rgba(current_idx) {
                let info = self.frame_info[current_idx];
                uploads.push(GifFrameUpload {
                    frame_index: current_idx,
                    rgba,
                    width: info.width,
                    height: info.height,
                });
            }
        }

        for frame_index in 0..frame_count {
            if uploads.len() >= GIF_MAX_UPLOADS_PER_FRAME {
                break;
            }
            if self.textures[frame_index].is_some() {
                continue;
            }
            if let Some(rgba) = data.take_frame_rgba(frame_index) {
                let info = self.frame_info[frame_index];
                uploads.push(GifFrameUpload {
                    frame_index,
                    rgba,
                    width: info.width,
                    height: info.height,
                });
            }
        }

        let has_pending_staging = data.frames.iter().any(|frame| frame.rgba.is_some());
        drop(data);

        let uploaded_count = uploads.len();
        for upload in uploads {
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [upload.width as usize, upload.height as usize],
                &upload.rgba,
            );
            let texture = ctx.load_texture(
                format!("gif_player_{}_{}", self.path.display(), upload.frame_index),
                color_image,
                egui::TextureOptions::LINEAR,
            );
            self.textures[upload.frame_index] = Some(texture);
        }

        if let Some(texture) = self.textures[current_idx].as_ref() {
            self.texture = Some(texture.clone());
            self.active_texture_frame = current_idx;
        } else if self.texture.is_none() {
            if let Some((frame_index, texture)) = self
                .textures
                .iter()
                .enumerate()
                .find_map(|(idx, texture)| texture.as_ref().map(|texture| (idx, texture)))
            {
                self.texture = Some(texture.clone());
                self.active_texture_frame = frame_index;
            }
        }

        if self.texture.is_none() {
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }

        let delay_ms = self
            .frame_info
            .get(self.current_frame)
            .map_or(100, |info| info.delay_ms);
        let delay = Duration::from_millis(delay_ms);

        if frame_count > 1 && self.last_update.elapsed() >= delay {
            let next_idx = (self.current_frame + 1) % frame_count;
            if let Some(texture) = self.textures[next_idx].as_ref() {
                self.current_frame = next_idx;
                self.texture = Some(texture.clone());
                self.active_texture_frame = next_idx;
                self.last_update = Instant::now();

                let next_delay = self
                    .frame_info
                    .get(next_idx)
                    .map_or(100, |info| info.delay_ms);
                ctx.request_repaint_after(Duration::from_millis(next_delay));
            } else {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        } else {
            let remaining = delay.saturating_sub(self.last_update.elapsed());
            ctx.request_repaint_after(remaining);
        }

        if !is_complete || has_pending_staging || uploaded_count >= GIF_MAX_UPLOADS_PER_FRAME {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    pub fn display_size(&self, max_size: egui::Vec2) -> egui::Vec2 {
        let Some(texture) = &self.texture else {
            return max_size;
        };

        let texture_size = texture.size_vec2();
        if texture_size.x <= 0.0 || texture_size.y <= 0.0 {
            return max_size;
        }

        let may_upscale = self
            .frame_info
            .get(self.active_texture_frame)
            .is_some_and(|frame| {
                frame.original_width > frame.width || frame.original_height > frame.height
            });

        let scale = (max_size.x / texture_size.x).min(max_size.y / texture_size.y);
        let scale = if may_upscale { scale } else { scale.min(1.0) };
        texture_size * scale.max(0.0)
    }
}
