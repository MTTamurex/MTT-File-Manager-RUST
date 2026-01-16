#![cfg(feature = "mpv-player")]

use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, CW_USEDEFAULT, SW_HIDE, SW_SHOW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE, WINDOW_EX_STYLE,
};
#[cfg(target_os = "windows")]
use windows::core::w;

/// Shared state for MPV playback (WIP parity with WebView2 state).
#[derive(Clone, Default)]
pub struct MpvState {
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_muted: bool,
}

/// MPV video preview (WIP). This is a scaffold for the migration.
pub struct MpvPreview {
    pub path: PathBuf,
    pub show_player: bool,
    pub play_on_init: bool,
    pub state: Arc<RwLock<MpvState>>,
    pub is_visible: bool,
    pub is_detached: bool,
    pub is_maximized: bool,
    pub fullscreen_applied: bool,
    pub prev_app_maximized: bool,
    pub restore_needed: bool,
    pub last_window_rect: Option<egui::Rect>,
    pub forced_size: Option<egui::Vec2>,
    pub last_mouse_activity: Option<Instant>,

    #[cfg(target_os = "windows")]
    mpv_hwnd: Option<HWND>,
    #[cfg(target_os = "windows")]
    main_hwnd: Option<HWND>,
    last_rect: egui::Rect,
    mpv: Option<Arc<mpv::Mpv>>,
    loaded_path: Option<PathBuf>,
}

impl MpvPreview {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            show_player: false,
            play_on_init: false,
            state: Arc::new(RwLock::new(MpvState {
                is_playing: false,
                current_time: 0.0,
                duration: 0.0,
                volume: 1.0,
                is_muted: false,
            })),
            is_visible: true,
            is_detached: false,
            is_maximized: false,
            fullscreen_applied: false,
            prev_app_maximized: false,
            restore_needed: false,
            last_window_rect: None,
            forced_size: None,
            last_mouse_activity: None,
            #[cfg(target_os = "windows")]
            mpv_hwnd: None,
            #[cfg(target_os = "windows")]
            main_hwnd: None,
            last_rect: egui::Rect::NAN,
            mpv: None,
            loaded_path: None,
        }
    }

    pub fn get_state(&self) -> MpvState {
        self.state.read().unwrap().clone()
    }

    pub fn play(&self) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("pause", false);
        }
    }

    pub fn pause(&self) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("pause", true);
        }
    }

    pub fn toggle_play(&mut self) {
        let is_playing = self.state.read().unwrap().is_playing;
        if is_playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn seek(&self, time: f64) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("time-pos", time.max(0.0));
        }
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("volume", (volume.clamp(0.0, 1.0) * 100.0) as f64);
            let _ = m.set_property("mute", false);
        }
        if let Ok(mut state) = self.state.write() {
            state.volume = volume.clamp(0.0, 1.0);
            state.is_muted = false;
        }
    }

    pub fn set_muted(&self, muted: bool) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("mute", muted);
        }
        if let Ok(mut state) = self.state.write() {
            state.is_muted = muted;
        }
    }

    pub fn toggle_mute(&self) {
        let muted = self.state.read().unwrap().is_muted;
        self.set_muted(!muted);
    }

    pub fn controls_active(&self) -> bool {
        self.last_mouse_activity
            .map(|t| t.elapsed() < Duration::from_secs(3))
            .unwrap_or(false)
    }

    #[cfg(target_os = "windows")]
    pub fn release_focus(&self, _main_hwnd: HWND) {
        // MPV does not capture focus like WebView2 by default.
    }

    #[cfg(target_os = "windows")]
    pub fn release_focus_auto(&self) {
        // No-op for MPV. Keep for API parity.
    }

    #[cfg(target_os = "windows")]
    pub fn has_hwnd(&self, hwnd: HWND) -> bool {
        self.mpv_hwnd.map_or(false, |h| h == hwnd)
    }

    pub fn update(&mut self, _ui: &mut egui::Ui, _frame: Option<&eframe::Frame>) {
        if !self.show_player {
            self.set_visibility(false);
            return;
        }

        let ui = _ui;

        // Reserve space for the video. If forced_size is set (detached mode with control bar), use it.
        let size = if let Some(forced) = self.forced_size {
            forced
        } else if self.is_detached {
            ui.available_size()
        } else {
            let available = ui.available_size();
            let preview_height = (available.x * 0.6).min(300.0);
            egui::vec2(available.x, preview_height)
        };
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

        // Track mouse activity for autohide controls
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pos) {
                self.last_mouse_activity = Some(Instant::now());
            }
        }

        // Init MPV and child window
        if self.mpv.is_none() {
            match mpv::Mpv::new() {
                Ok(m) => {
                    let m = Arc::new(m);
                    let _ = m.set_property("keep-open", "yes");
                    let _ = m.set_property("hwdec", "auto");
                    let _ = m.set_property("pause", true);
                    self.mpv = Some(m);
                }
                Err(e) => {
                    eprintln!("[MpvPreview] Failed to create MPV: {:?}", e);
                    return;
                }
            }
        }

        #[cfg(target_os = "windows")]
        {
            if self.main_hwnd.is_none() {
                if let Some(frame) = _frame {
                    use raw_window_handle::HasWindowHandle;
                    if let Ok(handle) = frame.window_handle() {
                        if let raw_window_handle::RawWindowHandle::Win32(wh) = handle.as_raw() {
                            let hwnd = HWND(wh.hwnd.get() as _);
                            if !hwnd.is_invalid() {
                                self.main_hwnd = Some(hwnd);
                            }
                        }
                    }
                }
            }

            if self.mpv_hwnd.is_none() {
                if let Some(parent) = self.main_hwnd {
                    unsafe {
                        let h_video = CreateWindowExW(
                            WINDOW_EX_STYLE::default(),
                            w!("static"),
                            w!(""),
                            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                            0,
                            0,
                            CW_USEDEFAULT,
                            CW_USEDEFAULT,
                            parent,
                            None,
                            None,
                            None,
                        )
                        .unwrap_or(HWND::default());

                        if !h_video.is_invalid() {
                            self.mpv_hwnd = Some(h_video);
                            if let Some(m) = &self.mpv {
                                let _ = m.set_property("wid", h_video.0 as i64);
                            }
                        }
                    }
                }
            }
        }

        // Load file once
        if self.loaded_path.as_ref() != Some(&self.path) {
            if let Some(m) = &self.mpv {
                let path_str = self.path.to_string_lossy().to_string();
                let _ = m.command("loadfile", &[&path_str]);
                if self.play_on_init {
                    let _ = m.set_property("pause", false);
                    self.play_on_init = false;
                }
            }
            self.loaded_path = Some(self.path.clone());
        }

        // Update playback state
        if let Some(m) = &self.mpv {
            if let Ok(pos) = m.get_property::<f64>("time-pos") {
                if let Ok(mut state) = self.state.write() {
                    state.current_time = pos;
                }
            }
            if let Ok(dur) = m.get_property::<f64>("duration") {
                if let Ok(mut state) = self.state.write() {
                    state.duration = dur;
                }
            }
            if let Ok(paused) = m.get_property::<bool>("pause") {
                if let Ok(mut state) = self.state.write() {
                    state.is_playing = !paused;
                }
            }
            if let Ok(vol) = m.get_property::<f64>("volume") {
                if let Ok(mut state) = self.state.write() {
                    state.volume = (vol / 100.0).clamp(0.0, 1.0) as f32;
                }
            }
            if let Ok(muted) = m.get_property::<bool>("mute") {
                if let Ok(mut state) = self.state.write() {
                    state.is_muted = muted;
                }
            }
        }

        // Move/resize child window
        if rect != self.last_rect {
            self.last_rect = rect;
        }

        #[cfg(target_os = "windows")]
        if let Some(h_video) = self.mpv_hwnd {
            let factor = ui.ctx().pixels_per_point();
            let x = (rect.min.x * factor) as i32;
            let y = (rect.min.y * factor) as i32;
            let w = (rect.width() * factor) as i32;
            let h = (rect.height() * factor) as i32;
            unsafe {
                let _ = MoveWindow(h_video, x, y, w.max(1), h.max(1), true);
            }
        }

        self.set_visibility(self.is_visible);
    }

    pub fn try_init(
        &mut self,
        _window: &dyn raw_window_handle::HasWindowHandle,
        _ctx: &egui::Context,
        _ui: &egui::Ui,
    ) {
        // MPV is initialized lazily in update()
    }

    pub fn is_initialized(&self) -> bool {
        self.mpv_hwnd.is_some()
    }

    pub fn set_visibility(&mut self, visible: bool) {
        self.is_visible = visible;
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.mpv_hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

pub fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as i64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:01}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

impl Drop for MpvPreview {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.mpv_hwnd.take() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}
