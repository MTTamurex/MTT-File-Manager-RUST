use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use serde_json;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, CW_USEDEFAULT, SW_HIDE, SW_SHOW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE, WINDOW_EX_STYLE,
};
#[cfg(target_os = "windows")]
use windows::core::w;

/// Track information for audio/subtitles.
#[derive(Clone, Debug, Default)]
pub struct TrackInfo {
    pub id: i64,
    pub track_type: String, // "audio", "video", "sub"
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
}

/// Shared state for MPV playback.
#[derive(Clone, Default)]
pub struct MpvState {
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_muted: bool,
    pub audio_tracks: Vec<TrackInfo>,
    pub subtitle_tracks: Vec<TrackInfo>,
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
    pub last_mouse_pos: Option<egui::Pos2>,
    /// Tracks if app was minimized to force window restoration
    pub was_minimized: bool,
    /// Tracks if NVIDIA VSR is currently enabled
    pub is_vsr_enabled: bool,

    #[cfg(target_os = "windows")]
    mpv_hwnd: Option<HWND>,
    #[cfg(target_os = "windows")]
    main_hwnd: Option<HWND>,
    last_rect: egui::Rect,
    mpv: Option<Arc<mpv::Mpv>>,
    loaded_path: Option<PathBuf>,
    pub video_menu: crate::ui::components::video_menu::VideoMenuState,
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
                audio_tracks: Vec::new(),
                subtitle_tracks: Vec::new(),
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
            last_mouse_pos: None,
            was_minimized: false,
            is_vsr_enabled: false,
            #[cfg(target_os = "windows")]
            mpv_hwnd: None,
            #[cfg(target_os = "windows")]
            main_hwnd: None,
            last_rect: egui::Rect::NAN,
            mpv: None,
            loaded_path: None,
            video_menu: Default::default(),
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

    pub fn set_audio_track(&self, id: i64) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("aid", id);
        }
    }

    pub fn set_subtitle_track(&self, id: i64) {
        if let Some(m) = &self.mpv {
            let _ = m.set_property("sid", id);
        }
    }

    #[cfg(target_os = "windows")]
    pub fn release_focus(&self, _main_hwnd: HWND) {
        // MPV does not capture focus by default.
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

        // Track mouse activity for autohide controls (movement-based)
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pos) {
                let moved = self
                    .last_mouse_pos
                    .map(|prev| prev.distance(pos) > 2.0)
                    .unwrap_or(true);
                if moved {
                    self.last_mouse_activity = Some(Instant::now());
                    self.last_mouse_pos = Some(pos);
                }
            }
        }

        // Init MPV and child window
        if self.mpv.is_none() {
            match mpv::Mpv::new() {
                Ok(m) => {
                    let m = Arc::new(m);
                    let _ = m.set_property("keep-open", "yes");
                    
                    // Mandatory configuration for NVIDIA RTX VSR
                    // We must use D3D11 backend and D3D11 VA hardware decoding
                    if let Err(e) = m.set_property("vo", "gpu") {
                        eprintln!("[MpvPreview] Failed to set vo=gpu: {:?}", e);
                    }
                    if let Err(e) = m.set_property("gpu-api", "d3d11") {
                        eprintln!("[MpvPreview] Failed to set gpu-api=d3d11: {:?}", e);
                    }
                    if let Err(e) = m.set_property("gpu-context", "d3d11") {
                        eprintln!("[MpvPreview] Failed to set gpu-context=d3d11: {:?}", e);
                    }
                    if let Err(e) = m.set_property("hwdec", "d3d11va") {
                        eprintln!("[MpvPreview] Failed to set hwdec=d3d11va: {:?}", e);
                    }

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
                            Some(parent),
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

            // Track list polling - Get as string first then parse JSON
            if let Ok(tracks_str) = m.get_property::<String>("track-list") {
                if let Ok(tracks_val) = serde_json::from_str::<serde_json::Value>(&tracks_str) {
                    if let Some(tracks_arr) = tracks_val.as_array() {
                        let mut audio_tracks = Vec::new();
                        let mut sub_tracks = Vec::new();

                        for t in tracks_arr {
                            let id = t["id"].as_i64().unwrap_or(0);
                            let t_type = t["type"].as_str().unwrap_or("").to_string();
                            let selected = t["selected"].as_bool().unwrap_or(false);
                            let title = t["title"].as_str().map(|s| s.to_string());
                            let lang = t["lang"].as_str().map(|s| s.to_string());

                            let info = TrackInfo {
                                id,
                                track_type: t_type.clone(),
                                title,
                                lang,
                                selected,
                            };

                            if t_type == "audio" {
                                audio_tracks.push(info);
                            } else if t_type == "sub" {
                                sub_tracks.push(info);
                            }
                        }

                        if let Ok(mut state) = self.state.write() {
                            state.audio_tracks = audio_tracks;
                            state.subtitle_tracks = sub_tracks;
                        }
                    }
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

        // Render Context Menu (native viewport, appears above MPV HWND)
        let action = {
            let state = self.state.read().unwrap();
            crate::ui::components::video_menu::render_video_menu(
                ui.ctx(),
                &mut self.video_menu,
                &state.audio_tracks,
                &state.subtitle_tracks,
                self.is_maximized,
            )
        };

        // Check for right-click context menu AFTER rendering
        // Always allow right-click to open/reposition the menu
        let right_click_pos = ui.ctx().input(|i| {
            if i.pointer.button_clicked(egui::PointerButton::Secondary) {
                i.pointer.latest_pos().or_else(|| i.pointer.hover_pos())
            } else {
                None
            }
        });

        if let Some(pos) = right_click_pos {
            if rect.contains(pos) {
                self.video_menu.active_submenu = None;
                self.video_menu.submenu_position = None;
                self.video_menu.main_menu_rect = None;
                self.video_menu.submenu_rect = None;
                self.video_menu.is_open = true;
                self.video_menu.position = pos;
                self.video_menu.menu_opened_at = Some(std::time::Instant::now());
            }
        }

        match action {
            crate::ui::components::video_menu::VideoMenuAction::None => {},
            crate::ui::components::video_menu::VideoMenuAction::TogglePlay => self.toggle_play(),
            crate::ui::components::video_menu::VideoMenuAction::ToggleMute => self.toggle_mute(),
            crate::ui::components::video_menu::VideoMenuAction::SetAudioTrack(id) => self.set_audio_track(id),
            crate::ui::components::video_menu::VideoMenuAction::SetSubtitleTrack(id) => self.set_subtitle_track(id),
            crate::ui::components::video_menu::VideoMenuAction::ToggleFullscreen => {
                // Toggle is handled externally - just set the flag
                self.is_maximized = !self.is_maximized;
            },
            crate::ui::components::video_menu::VideoMenuAction::Close => {
                 self.video_menu.is_open = false;
                 self.video_menu.active_submenu = None;
                 self.video_menu.submenu_position = None;
            },
            crate::ui::components::video_menu::VideoMenuAction::RightClickOutside(pos) => {
                // Menu was closed, now reopen at the provided position
                self.video_menu.is_open = true;
                self.video_menu.position = pos;
                self.video_menu.menu_opened_at = Some(std::time::Instant::now());
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

    /// Enables NVIDIA RTX Video Super Resolution (VSR).
    /// 
    /// Requires MPV to be initialized with:
    /// - vo=gpu
    /// - gpu-api=d3d11
    /// - hwdec=d3d11va
    pub fn enable_nvidia_vsr(&mut self) -> Result<(), String> {
        if let Some(m) = &self.mpv {
            // Apply the D3D11 Video Processing Post-processing filter
            // scale=2: Forces scaling to engage the driver
            // scaling-mode=nvidia: Explicitly requests NVIDIA algorithm
            m.set_property("vf", "d3d11vpp=scale=2:scaling-mode=nvidia")
                .map_err(|e| format!("Failed to enable VSR: {:?}", e))?;
            self.is_vsr_enabled = true;
            eprintln!("[MpvPreview] NVIDIA VSR Enabled");
            Ok(())
        } else {
            Err("MPV instance not initialized".to_string())
        }
    }

    /// Disables VSR by clearing the video filter chain.
    pub fn disable_vsr(&mut self) -> Result<(), String> {
        if let Some(m) = &self.mpv {
            m.set_property("vf", "")
                .map_err(|e| format!("Failed to disable VSR: {:?}", e))?;
            self.is_vsr_enabled = false;
            eprintln!("[MpvPreview] VSR Disabled");
            Ok(())
        } else {
            Err("MPV instance not initialized".to_string())
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
