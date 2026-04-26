use super::*;

impl MpvPreview {
    pub fn is_native_osc_active(&self) -> bool {
        self.osc_active
    }

    fn desired_osc_enabled(&self) -> bool {
        if !MPV_OSC_POC_ENABLED {
            return false;
        }
        // Disable native OSC for audio files — the egui control bar handles
        // playback and the OSC renders at wrong proportions on the small
        // waveform visualization surface.
        if let Some(p) = &self.loaded_path {
            let is_audio = p
                .extension()
                .and_then(|ext| ext.to_str())
                .map(crate::infrastructure::windows::is_audio_extension)
                .unwrap_or(false);
            if is_audio {
                return false;
            }
        }
        if MPV_OSC_POC_DETACHED_ONLY {
            return self.is_detached();
        }
        true
    }

    pub(super) fn sync_osc_runtime_state(&mut self, mpv: &mpv::Mpv) {
        let desired_custom_osc_visible = self.desired_osc_enabled();
        if self.last_osc_enabled != Some(desired_custom_osc_visible) {
            // Keep built-in OSC disabled and control only the custom script visibility.
            if let Err(e) = mpv.set_property("osc", false) {
                log::warn!("[MpvPreview] Failed to force osc=no : {:?}", e);
            }

            let visibility_mode = if desired_custom_osc_visible {
                "auto"
            } else {
                "never"
            };
            if let Err(e) = mpv.command("script-message", &["osc-visibility", visibility_mode, "1"])
            {
                log::warn!(
                    "[MpvPreview] Failed to set custom osc-visibility={} : {:?}",
                    visibility_mode,
                    e
                );
            }

            // Disable showonpause and idlescreen at the script level while
            // docked so the Lua handlers never override our "never" visibility.
            // Re-enable them when entering detached mode.
            if desired_custom_osc_visible {
                let _ = mpv.command(
                    "change-list",
                    &["script-opts", "append", "osc-showonpause=yes"],
                );
                let _ = mpv.command(
                    "change-list",
                    &["script-opts", "append", "osc-idlescreen=yes"],
                );
            } else {
                let _ = mpv.command(
                    "change-list",
                    &["script-opts", "append", "osc-showonpause=no"],
                );
                let _ = mpv.command(
                    "change-list",
                    &["script-opts", "append", "osc-idlescreen=no"],
                );
            }

            self.last_osc_enabled = Some(desired_custom_osc_visible);
        }

        let desired_fullscreen = self.is_fullscreen();
        if self.last_mpv_fullscreen != Some(desired_fullscreen) {
            if let Err(e) = mpv.set_property("fullscreen", desired_fullscreen) {
                log::warn!(
                    "[MpvPreview] Failed to set fullscreen={} : {:?}",
                    desired_fullscreen,
                    e
                );
            }
            self.last_mpv_fullscreen = Some(desired_fullscreen);
        }

        if !desired_custom_osc_visible {
            self.osc_pointer_inside = false;
            self.osc_primary_down = false;
            self.osc_secondary_down = false;
            self.osc_last_mouse_pos_px = None;
        }
        self.osc_active = desired_custom_osc_visible;
    }

    pub(super) fn sync_fullscreen_from_mpv(&mut self, ui: &egui::Ui, _mpv: &mpv::Mpv) {
        // PERF: Read fullscreen from shared state (polled by background event loop)
        let mpv_fullscreen = match self.state.try_read() {
            Ok(s) => s.fullscreen,
            Err(_) => return,
        };

        if self.last_observed_mpv_fullscreen == Some(mpv_fullscreen) {
            return;
        }
        self.last_observed_mpv_fullscreen = Some(mpv_fullscreen);

        // Map OSC fullscreen button to real app fullscreen transitions.
        if mpv_fullscreen && !self.is_fullscreen() && self.is_detached() {
            let was_maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            self.prev_app_maximized = was_maximized;
            self.mode = VideoMode::Fullscreen;
            self.fullscreen_applied = false;
        } else if !mpv_fullscreen && self.is_fullscreen() {
            // Mirror ESC fullscreen-exit flow to avoid viewport desync artifacts.
            self.mode = VideoMode::Detached;
            self.fullscreen_applied = false;
            self.restore_frames = 10;
            self.forced_size = None;
            self.reset_last_rect();
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            if self.prev_app_maximized {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
        }
    }

    pub(super) fn forward_osc_input(&mut self, ui: &egui::Ui, rect: egui::Rect, mpv: &mpv::Mpv) {
        let (hover_pos, primary_down, secondary_down, scroll_y) = ui.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.button_down(egui::PointerButton::Primary),
                i.pointer.button_down(egui::PointerButton::Secondary),
                i.raw_scroll_delta.y,
            )
        });

        let is_inside = hover_pos.map(|p| rect.contains(p)).unwrap_or(false);

        let current_mouse_px = if is_inside {
            hover_pos.map(|pos| {
                let factor = ui.ctx().pixels_per_point();
                let x = ((pos.x - rect.min.x) * factor).max(0.0) as i64;
                let y = ((pos.y - rect.min.y) * factor).max(0.0) as i64;
                (x, y)
            })
        } else {
            None
        };

        let moved = match (self.osc_last_mouse_pos_px, current_mouse_px) {
            (Some(prev), Some(cur)) => prev != cur,
            (None, Some(_)) => true,
            _ => false,
        };

        if is_inside && (moved || !self.osc_pointer_inside) {
            if let Some((x, y)) = current_mouse_px {
                let x_str = x.to_string();
                let y_str = y.to_string();
                let _ = mpv.command("mouse", &[x_str.as_str(), y_str.as_str()]);
            }
            let _ = mpv.command("keypress", &["MOUSE_MOVE"]);
        } else if self.osc_pointer_inside && !is_inside {
            let _ = mpv.command("keypress", &["MOUSE_LEAVE"]);
        }

        if primary_down != self.osc_primary_down {
            let cmd = if primary_down { "keydown" } else { "keyup" };
            let _ = mpv.command(cmd, &["MBTN_LEFT"]);
        }

        if secondary_down != self.osc_secondary_down {
            let cmd = if secondary_down { "keydown" } else { "keyup" };
            let _ = mpv.command(cmd, &["MBTN_RIGHT"]);
        }

        if is_inside {
            if let Some((x, y)) = current_mouse_px {
                let x_str = x.to_string();
                let y_str = y.to_string();
                let _ = mpv.command("mouse", &[x_str.as_str(), y_str.as_str()]);
            }
            if scroll_y > 0.0 {
                let _ = mpv.command("keypress", &["WHEEL_UP"]);
            } else if scroll_y < 0.0 {
                let _ = mpv.command("keypress", &["WHEEL_DOWN"]);
            }
        }

        self.osc_pointer_inside = is_inside;
        self.osc_last_mouse_pos_px = current_mouse_px;
        self.osc_primary_down = primary_down;
        self.osc_secondary_down = secondary_down;
    }
}
