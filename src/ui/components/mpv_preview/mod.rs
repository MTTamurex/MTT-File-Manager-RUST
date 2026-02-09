use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use windows::core::w;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, CW_USEDEFAULT, SW_HIDE, SW_SHOW,
    WINDOW_EX_STYLE, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

mod docked_filters;
mod lifecycle;
mod playback_state;
mod window_embed;

// Re-export from sub-modules for backward compatibility
use crate::ui::components::mpv::event_loop as mpv_event_loop;
use crate::ui::components::mpv::filters as mpv_filters;
use crate::ui::components::mpv::playback as mpv_playback;
pub use crate::ui::components::mpv::state::{MpvState, TrackInfo};
pub use crate::ui::components::mpv::utils::format_time;

const MPV_DEFAULT_CACHE_SECS: f64 = 8.0;
const MPV_DEFAULT_READAHEAD_SECS: f64 = 4.0;
const MPV_DEFAULT_DEMUXER_MAX_BYTES: i64 = 32_i64 * 1024 * 1024;
const MPV_DEFAULT_DEMUXER_MAX_BACK_BYTES: i64 = 8_i64 * 1024 * 1024;

const MPV_DOCKED_CACHE_SECS: f64 = 12.0;
const MPV_DOCKED_READAHEAD_SECS: f64 = 6.0;
const MPV_DOCKED_DEMUXER_MAX_BYTES: i64 = 48_i64 * 1024 * 1024;
const MPV_DOCKED_DEMUXER_MAX_BACK_BYTES: i64 = 12_i64 * 1024 * 1024;

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
    pub restore_frames: u8,
    pub last_window_rect: Option<egui::Rect>,
    pub forced_size: Option<egui::Vec2>,
    pub last_mouse_activity: Option<Instant>,
    pub last_mouse_pos: Option<egui::Pos2>,
    /// Tracks if app was minimized to force window restoration
    pub was_minimized: bool,
    /// Initial volume to apply when MPV is ready
    pub initial_volume: f32,
    /// Tracks if NVIDIA VSR is currently enabled
    pub is_vsr_enabled: bool,
    /// Tracks whether docked downscale is currently applied
    docked_downscale_applied: bool,
    /// Stores previous vf chain to restore on undock
    docked_prev_vf: Option<String>,
    /// Tracks whether docked FPS limiting is currently applied
    docked_fps_limit_applied: bool,
    /// Stores previous video-sync to restore on undock
    docked_prev_video_sync: Option<String>,
    /// Stores previous interpolation to restore on undock
    docked_prev_interpolation: Option<bool>,
    /// Stores previous tscale to restore on undock
    docked_prev_tscale: Option<String>,
    /// Stores previous cache setting to restore on undock
    docked_prev_cache: Option<String>,
    /// Stores previous cache-secs to restore on undock
    docked_prev_cache_secs: Option<f64>,
    /// Stores previous demuxer readahead to restore on undock
    docked_prev_readahead_secs: Option<f64>,
    /// Stores previous demuxer cache bytes to restore on undock
    docked_prev_demuxer_max_bytes: Option<i64>,
    /// Stores previous demuxer back cache bytes to restore on undock
    docked_prev_demuxer_max_back_bytes: Option<i64>,
    audio_normalizer_enabled: bool,
    last_deinterlace_check: Instant,

    // Performance: Async event handling (Fase 2 optimization)
    event_thread_running: Arc<AtomicBool>,
    event_thread_handle: Option<thread::JoinHandle<()>>,

    // Performance: Caching (polling removed in Fase 2)
    cached_duration: Option<f64>,
    cached_tracks: Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    pending_external_subtitle: Option<PathBuf>,

    #[cfg(target_os = "windows")]
    mpv_hwnd: Option<HWND>,
    #[cfg(target_os = "windows")]
    main_hwnd: Option<HWND>,
    last_rect: egui::Rect,
    mpv: Option<Arc<mpv::Mpv>>,
    loaded_path: Option<PathBuf>,
    pub controls_state: crate::ui::components::video_controls_state::VideoControlsState,
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
            restore_frames: 0,
            last_window_rect: None,
            forced_size: None,
            last_mouse_activity: None,
            last_mouse_pos: None,
            was_minimized: false,
            initial_volume: 1.0,
            is_vsr_enabled: false,
            docked_downscale_applied: false,
            docked_prev_vf: None,
            docked_fps_limit_applied: false,
            docked_prev_video_sync: None,
            docked_prev_interpolation: None,
            docked_prev_tscale: None,
            docked_prev_cache: None,
            docked_prev_cache_secs: None,
            docked_prev_readahead_secs: None,
            docked_prev_demuxer_max_bytes: None,
            docked_prev_demuxer_max_back_bytes: None,
            audio_normalizer_enabled: false,
            last_deinterlace_check: Instant::now(),
            event_thread_running: Arc::new(AtomicBool::new(false)),
            event_thread_handle: None,
            cached_duration: None,
            cached_tracks: None,
            pending_external_subtitle: None,
            #[cfg(target_os = "windows")]
            mpv_hwnd: None,
            #[cfg(target_os = "windows")]
            main_hwnd: None,
            last_rect: egui::Rect::NAN,
            mpv: None,
            loaded_path: None,
            controls_state: Default::default(),
        }
    }

    /// Reset the last rect to force window resize on next frame
    pub fn reset_last_rect(&mut self) {
        self.last_rect = egui::Rect::NAN;
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

                    // Use a balanced baseline profile for 4K stability.
                    // display-resample + interpolation can overload some GPUs in fullscreen.
                    let _ = m.set_property("video-sync", "audio");
                    let _ = m.set_property("interpolation", false);
                    let _ = m.set_property("tscale", "linear");
                    let _ = m.set_property("framedrop", "vo");

                    // Bound demux/cache memory so high-bitrate files do not balloon RAM usage.
                    let _ = m.set_property("cache", "yes");
                    let _ = m.set_property("cache-secs", MPV_DEFAULT_CACHE_SECS);
                    let _ = m.set_property("demuxer-readahead-secs", MPV_DEFAULT_READAHEAD_SECS);
                    let _ = m.set_property("demuxer-max-bytes", MPV_DEFAULT_DEMUXER_MAX_BYTES);
                    let _ = m
                        .set_property("demuxer-max-back-bytes", MPV_DEFAULT_DEMUXER_MAX_BACK_BYTES);

                    let _ = m.set_property("pause", true);

                    // PERF FASE 2: Start async event loop for push-based state updates
                    // This eliminates all FFI polling overhead (40 calls/sec → 0)
                    self.start_event_loop_internal(m.clone(), ui.ctx().clone());

                    self.mpv = Some(m);
                    self.set_audio_normalizer(self.audio_normalizer_enabled);

                    // Apply initial volume
                    self.set_volume(self.initial_volume);
                }
                Err(e) => {
                    eprintln!("[MpvPreview] Failed to create MPV: {:?}", e);
                    return;
                }
            }
        }

        self.ensure_main_hwnd_from_frame(_frame);
        self.ensure_mpv_hwnd_child();

        // Load file once
        if self.loaded_path.as_ref() != Some(&self.path) {
            if let Some(m) = &self.mpv {
                let path_str = self.path.to_string_lossy().to_string();
                let _ = m.command("loadfile", &[&path_str]);

                // Prefer sidecar subtitle when available (movie.srt, movie.en.srt, etc.)
                self.pending_external_subtitle = mpv_playback::find_sidecar_subtitle(&self.path);

                if self.play_on_init {
                    let _ = m.set_property("pause", false);
                    self.play_on_init = false;
                }
            }
            self.loaded_path = Some(self.path.clone());

            // Clear cached values for new file
            self.cached_duration = None;
            self.cached_tracks = None;
        }

        // Apply docked-mode downscale + FPS limit (dynamic, reversible, no player restart)
        if (!self.is_detached) != self.docked_downscale_applied
            || (!self.is_detached) != self.docked_fps_limit_applied
        {
            self.update_docked_downscale(false);
        }

        // PERF FASE 2: State updates now handled by async event loop (zero polling overhead!)
        // Only tracks still need manual fetching (heavy JSON parse, done once per file)
        // NOTE: We must wait for file to be loaded before querying tracks, otherwise we get empty list
        if let Some(m) = self.mpv.clone() {
            // Check if file is ready by checking if duration is available
            let file_ready = mpv_playback::is_file_ready(&m);
            if file_ready {
                if let Some(sidecar) = self.pending_external_subtitle.take() {
                    if let Err(e) = self.load_external_subtitle(&sidecar) {
                        eprintln!("[MPV] Failed to auto-load sidecar subtitle: {}", e);
                    }
                }
            }

            // CACHE: Track list (read once file is ready, then cache until file change)
            if self.cached_tracks.is_none() && file_ready {
                let (audio_tracks, sub_tracks): (Vec<TrackInfo>, Vec<TrackInfo>) =
                    mpv_playback::query_tracks(&m);

                // Cache the tracks (even if empty, file is loaded so this is final)
                self.cached_tracks = Some((audio_tracks.clone(), sub_tracks.clone()));

                if let Ok(mut state) = self.state.write() {
                    state.audio_tracks = audio_tracks;
                    state.subtitle_tracks = sub_tracks;
                }
            } else if let Some((ref audio, ref subs)) = self.cached_tracks {
                // Use cached tracks
                if let Ok(mut state) = self.state.write() {
                    state.audio_tracks = audio.clone();
                    state.subtitle_tracks = subs.clone();
                }
            }
        }

        if self.last_deinterlace_check.elapsed() >= Duration::from_millis(500) {
            self.update_deinterlace_filter();
            self.last_deinterlace_check = Instant::now();
        }

        self.sync_child_window_rect(ui, rect);

        // Context menu removed - controls now in control bar
        // Double-click to toggle fullscreen is handled in preview_panel.rs

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
        // Não desligar mais o vídeo - apenas controlar visibilidade da janela
        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.mpv_hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }
}

impl Drop for MpvPreview {
    fn drop(&mut self) {
        self.shutdown();
    }
}
