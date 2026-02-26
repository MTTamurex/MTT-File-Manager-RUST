//! Dedicated video player application (standalone process).
//!
//! Runs MPV in a native eframe window with the MPV native OSC for controls.
//! Supports fullscreen toggle, initial seek position, and volume.

use crate::ui::components::mpv_preview::{MpvPreview, VideoMode};
use eframe::egui;
use std::path::PathBuf;

pub struct DedicatedVideoPlayerApp {
    player: MpvPreview,
    initial_position: f64,
    position_applied: bool,
    startup_frame: u32,
}

impl DedicatedVideoPlayerApp {
    pub fn new(path: PathBuf, position: f64, volume: f32) -> Self {
        let mut player = MpvPreview::new(path);
        player.play_on_init = true;
        player.show_player = true;
        player.initial_volume = volume;
        // Set to Detached mode so OSC is active and forced_size uses available space
        player.mode = VideoMode::Detached;

        Self {
            player,
            initial_position: position,
            position_applied: false,
            startup_frame: 0,
        }
    }
}

impl eframe::App for DedicatedVideoPlayerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.startup_frame = self.startup_frame.saturating_add(1);

        // Apply initial seek position once MPV has loaded the file and duration is known
        if !self.position_applied && self.initial_position > 0.5 {
            let state = self.player.get_state();
            if state.duration > 0.0 {
                self.player.seek(self.initial_position);
                self.position_applied = true;
            }
        }

        // Sync fullscreen state between MPV and the eframe viewport.
        // If the user toggles fullscreen via MPV OSC (e.g. double-click or 'f'),
        // we propagate that to the eframe window.
        {
            let state = self.player.get_state();
            let egui_fullscreen = ctx.input(|i| {
                i.viewport()
                    .fullscreen
                    .unwrap_or(false)
            });

            if state.fullscreen && !egui_fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            } else if !state.fullscreen && egui_fullscreen {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
        }

        // Render the MPV player in a full CentralPanel
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                let available = ui.available_size();
                self.player.forced_size = Some(available);
                self.player.update(ui, Some(frame));
            });

        // Keep repainting while video is playing
        let is_playing = self
            .player
            .state
            .try_read()
            .map(|s| s.is_playing)
            .unwrap_or(false);

        if is_playing {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.player.shutdown();
    }
}
