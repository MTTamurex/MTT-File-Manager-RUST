use super::*;
use std::path::Path;

impl MpvPreview {
    /// Returns the current state safely, with default value on error
    pub fn get_state(&self) -> MpvState {
        match self.state.read() {
            Ok(state) => MpvState::clone(&state),
            Err(_) => {
                log::error!("[MpvPreview] Erro ao ler estado - RwLock poisonado");
                MpvState::default()
            }
        }
    }

    /// Tries to get the state with explicit error handling
    pub fn try_get_state(&self) -> Result<MpvState, String> {
        self.state
            .read()
            .map(|state: std::sync::RwLockReadGuard<'_, MpvState>| MpvState::clone(&state))
            .map_err(|e| format!("[MpvPreview] RwLock poisonado: {}", e))
    }

    pub fn play(&self) {
        mpv_playback::play(&self.mpv);
    }

    pub fn pause(&self) {
        mpv_playback::pause(&self.mpv);
    }

    pub fn toggle_play(&mut self) {
        let was_playing = match self.state.read() {
            Ok(state) => state.is_playing,
            Err(_) => {
                log::error!("[MpvPreview] Erro ao toggle play - RwLock poisonado");
                self.pause();
                return;
            }
        };

        if was_playing {
            self.pause();
        } else {
            self.play();
        }

        // Immediately update state so UI reflects the change without waiting for event loop
        if let Ok(mut s) = self.state.try_write() {
            s.is_playing = !was_playing;
        }

        // In docked mode, immediately suppress OSC and reset tracking so
        // sync_osc_runtime_state re-sends on next frame (double-tap suppression).
        if self.is_docked() {
            if let Some(m) = &self.mpv {
                let _ = m.command("script-message", &["osc-visibility", "never", "1"]);
            }
        }
    }

    pub fn seek(&self, time: f64) {
        mpv_playback::seek(&self.mpv, time);
    }

    pub fn seek_relative(&self, delta_seconds: f64) {
        mpv_playback::seek_relative(&self.mpv, delta_seconds);
    }

    pub fn set_volume(&self, volume: f32) {
        let clamped = volume.clamp(0.0, 1.0);
        if let Some(m) = &self.mpv {
            let _ = m.set_property("volume", (clamped * 100.0) as f64);
            let _ = m.set_property("mute", false);
        }
        if let Ok(mut state) = self.state.write() {
            state.volume = clamped;
            state.is_muted = false;
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("mute", muted);
        }
        if let Ok(mut state) = self.state.try_write() {
            state.is_muted = muted;
        }
    }

    pub fn toggle_mute(&self) {
        let current_muted = match self.state.try_read() {
            Ok(state) => state.is_muted,
            Err(_) => {
                log::error!("[MpvPreview] Erro ao ler estado mute - RwLock poisonado ou ocupado");
                false
            }
        };

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.set_muted(!current_muted);
        }));
    }

    /// Show OSD text on the video using MPV's native show-text command
    pub fn show_osd_text(&self, text: &str, duration_ms: i64) {
        if let Some(m) = &self.mpv {
            let dur_str = duration_ms.to_string();
            let _ = m.command("show-text", &[text, &dur_str]);
        }
    }

    pub fn controls_active(&self) -> bool {
        self.last_mouse_activity
            .map(|t| t.elapsed() < Duration::from_secs(3))
            .unwrap_or(false)
    }

    /// Returns the current display aspect ratio reported by MPV.
    /// PERF: Reads from cached state (polled by background event loop).
    pub fn video_aspect(&self) -> Option<f64> {
        self.state.try_read().ok().and_then(|s| s.video_aspect)
    }

    pub fn toggle_audio_normalizer(&mut self) {
        let enabled = !self.audio_normalizer_enabled;
        self.set_audio_normalizer(enabled);
    }

    pub fn is_audio_normalizer_enabled(&self) -> bool {
        self.audio_normalizer_enabled
    }

    pub(super) fn set_audio_normalizer(&mut self, enabled: bool) {
        if let Some(m) = &self.mpv {
            let current_af = m.get_property::<String>("af").unwrap_or_default();
            let has_normalizer = current_af.contains(mpv_filters::AUDIO_NORMALIZER_MARKER);
            let next_af = if enabled && !has_normalizer {
                mpv_filters::append_af_filter(&current_af, mpv_filters::AUDIO_NORMALIZER_FILTER)
            } else if !enabled && has_normalizer {
                mpv_filters::remove_af_filter(&current_af, mpv_filters::AUDIO_NORMALIZER_MARKER)
            } else {
                current_af
            };
            let _ = m.set_property("af", next_af);
        }
        self.audio_normalizer_enabled = enabled;
    }

    pub fn set_audio_track(&mut self, id: i64) {
        mpv_playback::set_audio_track(&self.mpv, &self.state, &mut self.cached_tracks, id);
    }

    pub fn set_subtitle_track(&mut self, id: i64) {
        mpv_playback::set_subtitle_track(&self.mpv, &self.state, &mut self.cached_tracks, id);
    }

    pub fn load_external_subtitle(&mut self, subtitle_path: &Path) -> Result<(), String> {
        mpv_playback::load_external_subtitle(
            &self.mpv,
            &self.state,
            &mut self.cached_tracks,
            subtitle_path,
        )
    }
}
