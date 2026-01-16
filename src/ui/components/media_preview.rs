use eframe::egui;
use std::time::{Duration, Instant};
use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;

use super::mpv_preview::{format_time as backend_format_time, MpvPreview as VideoPreview, MpvState as VideoState};

// ============================================================================
// GIF Player (Mantido inalterado)
// ============================================================================

#[derive(Clone)]
pub struct GifFrame {
    pub texture: egui::TextureHandle,
    pub delay: Duration,
}

#[derive(Clone)]
pub struct GifPlayer {
    pub frames: Vec<GifFrame>,
    pub current_frame: usize,
    pub last_update: Instant,
    pub total_duration: Duration,
}

impl GifPlayer {
    pub fn load(ctx: &egui::Context, path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let decoder = GifDecoder::new(reader)?;
        let frames_result = decoder.into_frames().collect_frames()?;

        let mut gif_frames = Vec::new();
        let mut total_duration = Duration::ZERO;

        for (i, frame) in frames_result.into_iter().enumerate() {
            let (numerator, denominator): (u32, u32) = frame.delay().numer_denom_ms();
            let delay = if denominator == 0 {
                Duration::from_millis(100)
            } else {
                Duration::from_millis((numerator as u64) / (denominator as u64))
            };

            let buffer = frame.into_buffer();
            let (width, height) = buffer.dimensions();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                buffer.as_flat_samples().as_slice(),
            );

            let texture = ctx.load_texture(
                format!("gif_frame_{}_{}", path.display(), i),
                color_image,
                Default::default(),
            );

            total_duration += delay;
            gif_frames.push(GifFrame { texture, delay });
        }

        if gif_frames.is_empty() {
            return Err("GIF has no frames".into());
        }

        Ok(Self {
            frames: gif_frames,
            current_frame: 0,
            last_update: Instant::now(),
            total_duration,
        })
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        let current_delay = self.frames[self.current_frame].delay;

        if elapsed >= current_delay {
            self.current_frame = (self.current_frame + 1) % self.frames.len();
            self.last_update = now;
            ctx.request_repaint_after(self.frames[self.current_frame].delay);
        } else {
            ctx.request_repaint_after(current_delay - elapsed);
        }
    }

    pub fn current_texture(&self) -> &egui::TextureHandle {
        &self.frames[self.current_frame].texture
    }
}

// ============================================================================
// Media Preview Enum
// ============================================================================

pub enum MediaPreview {
    StaticImage(egui::TextureHandle),
    AnimatedGif(GifPlayer),
    Video(VideoPreview),
    Error(String),
}

impl MediaPreview {
    pub fn show(&mut self, ui: &mut egui::Ui, frame: Option<&eframe::Frame>) -> egui::Response {
        match self {
            MediaPreview::StaticImage(texture) => {
                let max_size = egui::vec2(ui.available_width(), ui.available_height());
                ui.add(egui::Image::new(&*texture).max_size(max_size).shrink_to_fit())
            }
            MediaPreview::AnimatedGif(player) => {
                player.update(ui.ctx());
                let texture = player.current_texture();
                let max_size = egui::vec2(ui.available_width(), ui.available_height());
                ui.add(egui::Image::new(texture).max_size(max_size).shrink_to_fit())
            }
            MediaPreview::Video(player) => {
                player.update(ui, frame);
                // Return a minimal response - the video preview already allocated its space
                ui.allocate_response(egui::vec2(0.0, 0.0), egui::Sense::hover()) 
            }
            MediaPreview::Error(msg) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                }).response
            }
        }
    }
    
    // ========================================================================
    // Video control methods (delegate to MPV preview)
    // ========================================================================
    
    /// Check if this is a video preview
    pub fn is_video(&self) -> bool {
        matches!(self, MediaPreview::Video(_))
    }
    
    /// Check if video player is showing (not just thumbnail)
    pub fn is_player_visible(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.show_player
        } else {
            false
        }
    }
    
    /// Get video playback state
    pub fn get_video_state(&self) -> Option<VideoState> {
        if let MediaPreview::Video(player) = self {
            Some(player.get_state())
        } else {
            None
        }
    }
    
    /// Toggle play/pause and show player if needed
    pub fn toggle_play(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.show_player = true;
            player.toggle_play();
        }
    }
    
    /// Start playing video
    pub fn play(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.show_player = true;
            player.play();
        }
    }
    
    /// Pause video
    pub fn pause(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.pause();
        }
    }
    
    /// Seek to specific time
    pub fn seek(&self, time: f64) {
        if let MediaPreview::Video(player) = self {
            player.seek(time);
        }
    }
    
    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        if let MediaPreview::Video(player) = self {
            player.set_volume(volume);
        }
    }
    
    /// Toggle mute
    pub fn toggle_mute(&self) {
        if let MediaPreview::Video(player) = self {
            player.toggle_mute();
        }
    }

    pub fn set_audio_track(&self, id: i64) {
        if let MediaPreview::Video(player) = self {
            player.set_audio_track(id);
        }
    }

    pub fn set_subtitle_track(&self, id: i64) {
        if let MediaPreview::Video(player) = self {
            player.set_subtitle_track(id);
        }
    }

    /// Whether video controls should be visible (based on mouse activity)
    pub fn controls_active(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.controls_active()
        } else {
            false
        }
    }

    /// Check if video is detached
    pub fn is_detached(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.is_detached
        } else {
            false
        }
    }

    /// Set detached state
    pub fn set_detached(&mut self, detached: bool) {
        if let MediaPreview::Video(player) = self {
            player.is_detached = detached;
            if !detached {
                player.is_maximized = false;
                player.forced_size = None;
            }
        }
    }

    /// Toggle detached state
    pub fn toggle_detached(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.is_detached = !player.is_detached;
            // Reset maximize state and forced_size when re-attaching
            if !player.is_detached {
                player.is_maximized = false;
                player.forced_size = None;
            }
        }
    }

    /// Check if video is maximized
    pub fn is_maximized(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.is_maximized
        } else {
            false
        }
    }

    /// Track whether app was maximized before fullscreen
    pub fn set_prev_app_maximized(&mut self, value: bool) {
        if let MediaPreview::Video(player) = self {
            player.prev_app_maximized = value;
        }
    }

    pub fn prev_app_maximized(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.prev_app_maximized
        } else {
            false
        }
    }

    /// Track whether fullscreen command has been applied
    pub fn set_fullscreen_applied(&mut self, value: bool) {
        if let MediaPreview::Video(player) = self {
            player.fullscreen_applied = value;
        }
    }

    pub fn fullscreen_applied(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.fullscreen_applied
        } else {
            false
        }
    }

    /// Toggle maximized state
    pub fn toggle_maximized(&mut self) {
        if let MediaPreview::Video(player) = self {
            if player.is_maximized {
                // Going from Maximized -> Normal
                player.is_maximized = false;
                player.restore_needed = true;
            } else {
                // Going from Normal -> Maximized
                player.is_maximized = true;
                // No restore needed logic here, resizing handled by fixed_rect
            }
        }
    }

    pub fn should_restore(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.restore_needed
        } else {
            false
        }
    }

    pub fn complete_restore(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.restore_needed = false;
        }
    }

    /// Set last window rect (for restore)
    pub fn set_last_window_rect(&mut self, rect: egui::Rect) {
        if let MediaPreview::Video(player) = self {
            player.last_window_rect = Some(rect);
        }
    }

    /// Get last window rect
    pub fn get_last_window_rect(&self) -> Option<egui::Rect> {
        if let MediaPreview::Video(player) = self {
            player.last_window_rect
        } else {
            None
        }
    }

    pub fn set_forced_size(&mut self, size: Option<egui::Vec2>) {
        if let MediaPreview::Video(player) = self {
            player.forced_size = size;
        }
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            MediaPreview::StaticImage(_) => None,
            MediaPreview::AnimatedGif(_) => None, // GIF player doesn't currently store path, but we could add it
            MediaPreview::Video(player) => Some(&player.path),
            MediaPreview::Error(_) => None,
        }
    }
}

pub fn format_time(seconds: f64) -> String {
    backend_format_time(seconds)
}
