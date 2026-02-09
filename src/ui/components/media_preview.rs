use crate::ui::components::gif_manager::GifData;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::mpv_preview::{
    format_time as backend_format_time, MpvPreview as VideoPreview, MpvState as VideoState,
};

// ============================================================================
// GIF Player (Mantido inalterado)
// ============================================================================

#[derive(Clone)]
pub struct GifPlayer {
    pub path: PathBuf,
    pub data: Arc<Mutex<GifData>>,
    pub texture: Option<egui::TextureHandle>,
    pub current_frame: usize,
    pub last_update: Instant,
}

impl GifPlayer {
    pub fn new(path: PathBuf, data: Arc<Mutex<GifData>>) -> Self {
        Self {
            path,
            data,
            texture: None,
            current_frame: 0,
            last_update: Instant::now(),
        }
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        let (frame_to_show, delay_ms, is_complete) = {
            match self.data.lock() {
                Ok(d) => {
                    if d.frames.is_empty() {
                        return;
                    }

                    let frame_idx = self.current_frame % d.frames.len();
                    let frame = &d.frames[frame_idx];
                    (Some(frame.clone()), frame.delay_ms, d.is_complete)
                }
                Err(_) => {
                    eprintln!("[GifPlayer] Erro ao lock dados - Mutex poisonado");
                    return;
                }
            }
        };

        if let Some(frame) = frame_to_show {
            // Initial texture creation or update
            if self.texture.is_none() {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.rgba,
                );
                self.texture = Some(ctx.load_texture(
                    format!("gif_player_{}", self.path.display()),
                    color_image,
                    Default::default(),
                ));
            } else if self.last_update.elapsed() >= Duration::from_millis(delay_ms) {
                // Update existing texture content
                self.current_frame += 1;
                let next_idx = {
                    match self.data.lock() {
                        Ok(d) => self.current_frame % d.frames.len(),
                        Err(_) => {
                            eprintln!(
                                "[GifPlayer] Erro ao lock dados para next_idx - Mutex poisonado"
                            );
                            return;
                        }
                    }
                };

                let next_frame = {
                    match self.data.lock() {
                        Ok(d) => d.frames[next_idx].clone(),
                        Err(_) => {
                            eprintln!(
                                "[GifPlayer] Erro ao lock dados para next_frame - Mutex poisonado"
                            );
                            return;
                        }
                    }
                };

                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [next_frame.width as usize, next_frame.height as usize],
                    &next_frame.rgba,
                );

                if let Some(tex) = &mut self.texture {
                    tex.set(color_image, Default::default());
                }

                self.last_update = Instant::now();
                ctx.request_repaint_after(Duration::from_millis(next_frame.delay_ms));
            } else {
                // Not yet time for next frame
                let remaining =
                    Duration::from_millis(delay_ms).saturating_sub(self.last_update.elapsed());
                ctx.request_repaint_after(remaining);
            }
        }

        // If not complete, keep checking for new frames
        if !is_complete {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
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
                ui.add(
                    egui::Image::new(&*texture)
                        .max_size(max_size)
                        .shrink_to_fit(),
                )
            }
            MediaPreview::AnimatedGif(player) => {
                player.update(ui.ctx());
                if let Some(texture) = player.texture() {
                    let max_size = egui::vec2(ui.available_width(), ui.available_height());
                    ui.add(egui::Image::new(texture).max_size(max_size).shrink_to_fit())
                } else {
                    ui.spinner()
                }
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
                })
                .response
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

    pub fn is_visible(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.is_visible
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

    /// Explicitly teardown the underlying player resources.
    pub fn shutdown(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.shutdown();
        }
    }

    /// Seek to specific time
    pub fn seek(&self, time: f64) {
        if let MediaPreview::Video(player) = self {
            player.seek(time);
        }
    }

    /// Seek relative to current position
    pub fn seek_relative(&self, delta_seconds: f64) {
        if let MediaPreview::Video(player) = self {
            player.seek_relative(delta_seconds);
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
            // CRASH FIX: Wrap in catch_unwind to prevent crash propagation
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                player.toggle_mute();
            }));
        }
    }

    pub fn set_audio_track(&mut self, id: i64) {
        if let MediaPreview::Video(player) = self {
            player.set_audio_track(id);
        }
    }

    pub fn set_subtitle_track(&mut self, id: i64) {
        if let MediaPreview::Video(player) = self {
            player.set_subtitle_track(id);
        }
    }

    pub fn load_external_subtitle(
        &mut self,
        subtitle_path: &std::path::Path,
    ) -> Result<(), String> {
        if let MediaPreview::Video(player) = self {
            player.load_external_subtitle(subtitle_path)
        } else {
            Err("Preview atual não é vídeo".to_string())
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

    /// Reset mouse activity timer to keep controls visible
    pub fn reset_mouse_activity(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.last_mouse_activity = Some(std::time::Instant::now());
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

    /// Get native handle for video window
    #[cfg(target_os = "windows")]
    pub fn get_hwnd(&self) -> Option<windows::Win32::Foundation::HWND> {
        if let MediaPreview::Video(player) = self {
            player.get_hwnd()
        } else {
            None
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
                player.restore_frames = 10;
            } else {
                // Going from Normal -> Maximized
                player.is_maximized = true;
                // No restore needed logic here, resizing handled by fixed_rect
            }
        }
    }

    pub fn should_restore(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.restore_frames > 0
        } else {
            false
        }
    }

    pub fn complete_restore(&mut self) {
        if let MediaPreview::Video(player) = self {
            if player.restore_frames > 0 {
                player.restore_frames -= 1;
            }
        }
    }

    /// Set restore needed flag
    pub fn set_restore_needed(&mut self, needed: bool) {
        if let MediaPreview::Video(player) = self {
            player.restore_frames = if needed { 10 } else { 0 };
        }
    }

    /// Check if was minimized
    pub fn was_minimized(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.was_minimized
        } else {
            false
        }
    }

    /// Set was minimized flag
    pub fn set_was_minimized(&mut self, minimized: bool) {
        if let MediaPreview::Video(player) = self {
            player.was_minimized = minimized;
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

    /// Reset the last rect to force MPV window resize on next frame
    pub fn reset_last_rect(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.reset_last_rect();
        }
    }

    /// Check if VSR is enabled
    pub fn is_vsr_enabled(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.is_vsr_enabled
        } else {
            false
        }
    }

    /// Toggle NVIDIA VSR
    pub fn toggle_vsr(&mut self) -> Result<(), String> {
        if let MediaPreview::Video(player) = self {
            if player.is_vsr_enabled {
                player.disable_vsr()
            } else {
                player.enable_nvidia_vsr()
            }
        } else {
            Ok(())
        }
    }

    /// Force video restore when stuck on white screen - only works for video player
    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            MediaPreview::StaticImage(_) => None,
            MediaPreview::AnimatedGif(_) => None, // GIF player doesn't currently store path, but we could add it
            MediaPreview::Video(player) => Some(&player.path),
            MediaPreview::Error(_) => None,
        }
    }

    /// Check if audio normalizer is enabled
    pub fn is_audio_normalizer_enabled(&self) -> bool {
        if let MediaPreview::Video(player) = self {
            player.is_audio_normalizer_enabled()
        } else {
            false
        }
    }

    /// Toggle audio normalizer
    pub fn toggle_audio_normalizer(&mut self) {
        if let MediaPreview::Video(player) = self {
            player.toggle_audio_normalizer();
        }
    }

    /// Access to controls state (for inline menus)
    pub fn controls_state_mut(
        &mut self,
    ) -> Option<&mut crate::ui::components::video_controls_state::VideoControlsState> {
        if let MediaPreview::Video(player) = self {
            Some(&mut player.controls_state)
        } else {
            None
        }
    }
}

pub fn format_time(seconds: f64) -> String {
    backend_format_time(seconds)
}
