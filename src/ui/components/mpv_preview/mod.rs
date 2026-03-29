use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

mod docked_filters;
mod lifecycle;
mod osc_input;
mod playback_state;
mod update_loop;
mod window_embed;

pub use window_embed::VideoSurface;

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
const MPV_OSC_POC_ENABLED: bool = true;
const MPV_OSC_POC_DETACHED_ONLY: bool = true;
const MPV_OSC_POC_SCRIPT_OPTS: &str = "osc-scalewindowed=1.8,osc-scalefullscreen=2.8";

/// Represents the current display mode of the video player.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoMode {
    /// Embedded in the right sidebar preview panel
    Docked,
    /// Floating egui::Window
    Detached,
    /// Full viewport
    Fullscreen,
}

/// MPV video preview component.
pub struct MpvPreview {
    pub path: PathBuf,
    pub show_player: bool,
    pub play_on_init: bool,
    pub state: Arc<RwLock<MpvState>>,
    pub is_visible: bool,
    /// Current display mode of the video player
    pub mode: VideoMode,
    pub fullscreen_applied: bool,
    pub prev_app_maximized: bool,
    pub restore_frames: u8,
    pub last_window_rect: Option<egui::Rect>,
    pub forced_size: Option<egui::Vec2>,
    pub last_mouse_activity: Option<Instant>,
    pub last_mouse_pos: Option<egui::Pos2>,
    /// POC OSC: track pointer state to forward input events to MPV when embedded.
    osc_pointer_inside: bool,
    osc_primary_down: bool,
    osc_secondary_down: bool,
    osc_last_mouse_pos_px: Option<(i64, i64)>,
    osc_active: bool,
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

    // Performance: Async event handling (Fase 2 optimization)
    event_thread_running: Arc<AtomicBool>,
    event_thread_handle: Option<thread::JoinHandle<()>>,

    // Performance: Caching (polling removed in Fase 2)
    cached_duration: Option<f64>,
    cached_tracks: Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    pending_external_subtitle: Option<PathBuf>,

    // PERF: Shared signal for background track querying
    tracks_need_query: Arc<AtomicBool>,
    // PERF: Gate event loop writes during file transitions
    file_loading: Arc<AtomicBool>,
    // PERF: Async sidecar subtitle search receiver
    sidecar_rx: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
    // PERF: Track previous interlaced state for change detection
    last_interlaced: Option<bool>,
    // PERF: Track last play state to minimize osc-visibility IPC commands
    osc_last_playing_for_suppress: Option<bool>,

    /// Native window surface for video rendering (encapsulates all HWND logic)
    pub surface: VideoSurface,
    mpv: Option<Arc<mpv::Mpv>>,
    loaded_path: Option<PathBuf>,
    last_osc_enabled: Option<bool>,
    last_observed_mpv_fullscreen: Option<bool>,
    last_mpv_fullscreen: Option<bool>,
    pub controls_state: crate::ui::components::video_controls_state::VideoControlsState,
}

impl MpvPreview {
    fn mpv_path_string(path: &std::path::Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn resolve_mpv_ui_config_dir() -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();

        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("mpv_ui").join("portable_config"));
        }

        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join("mpv_ui").join("portable_config"));
                candidates.push(exe_dir.join("..").join("mpv_ui").join("portable_config"));
                candidates.push(
                    exe_dir
                        .join("..")
                        .join("..")
                        .join("mpv_ui")
                        .join("portable_config"),
                );
            }
        }

        candidates
            .into_iter()
            .find(|dir| dir.join("scripts").join("modernH.lua").is_file())
    }

    fn create_mpv_instance() -> Result<mpv::Mpv, mpv::Error> {
        if MPV_OSC_POC_ENABLED {
            let config_dir = Self::resolve_mpv_ui_config_dir();
            if config_dir.is_none() {
                log::warn!(
                    "[MpvPreview] MPV UI folder not found (expected mpv_ui/portable_config with scripts/modernH.lua)"
                );
            }

            mpv::Mpv::with_initializer(|init| {
                // POC: load MPV UI assets from local folder and keep MPV default input bindings.
                if let Err(e) = init.set_option("load-scripts", true) {
                    log::warn!("[MpvPreview] Failed to set load-scripts=yes: {:?}", e);
                }
                if let Err(e) = init.set_option("osc", false) {
                    log::warn!("[MpvPreview] Failed to set osc=no: {:?}", e);
                }
                if let Err(e) = init.set_option("input-default-bindings", true) {
                    log::warn!(
                        "[MpvPreview] Failed to set input-default-bindings=yes: {:?}",
                        e
                    );
                }
                if let Err(e) = init.set_option("input-vo-keyboard", true) {
                    log::warn!("[MpvPreview] Failed to set input-vo-keyboard=yes: {:?}", e);
                }
                if let Err(e) = init.set_option("input-cursor", true) {
                    log::warn!("[MpvPreview] Failed to set input-cursor=yes: {:?}", e);
                }
                if let Err(e) = init.set_option("cursor-autohide", 1000_i64) {
                    log::warn!("[MpvPreview] Failed to set cursor-autohide=1000: {:?}", e);
                }
                if let Err(e) = init.set_option("script-opts", MPV_OSC_POC_SCRIPT_OPTS) {
                    log::warn!(
                        "[MpvPreview] Failed to set script-opts={} : {:?}",
                        MPV_OSC_POC_SCRIPT_OPTS,
                        e
                    );
                }

                if let Some(dir) = &config_dir {
                    let dir_str = Self::mpv_path_string(dir.as_path());
                    if let Err(e) = init.set_option("config", true) {
                        log::warn!("[MpvPreview] Failed to set config=yes: {:?}", e);
                    }
                    if let Err(e) = init.set_option("config-dir", dir_str.as_str()) {
                        log::warn!(
                            "[MpvPreview] Failed to set config-dir={} : {:?}",
                            dir_str,
                            e
                        );
                    }

                    let osc_script = dir.join("scripts").join("modernH.lua");
                    if !osc_script.is_file() {
                        log::warn!(
                            "[MpvPreview] modernH.lua not found at {}",
                            osc_script.to_string_lossy()
                        );
                    }
                }
                Ok(())
            })
        } else {
            mpv::Mpv::new()
        }
    }

    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            show_player: false,
            play_on_init: false,
            state: Arc::new(RwLock::new(MpvState {
                volume: 1.0,
                ..Default::default()
            })),
            is_visible: true,
            mode: VideoMode::Docked,
            fullscreen_applied: false,
            prev_app_maximized: false,
            restore_frames: 0,
            last_window_rect: None,
            forced_size: None,
            last_mouse_activity: None,
            last_mouse_pos: None,
            osc_pointer_inside: false,
            osc_primary_down: false,
            osc_secondary_down: false,
            osc_last_mouse_pos_px: None,
            osc_active: false,
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
            event_thread_running: Arc::new(AtomicBool::new(false)),
            event_thread_handle: None,
            cached_duration: None,
            cached_tracks: None,
            pending_external_subtitle: None,
            tracks_need_query: Arc::new(AtomicBool::new(false)),
            file_loading: Arc::new(AtomicBool::new(false)),
            sidecar_rx: None,
            last_interlaced: None,
            osc_last_playing_for_suppress: None,
            surface: VideoSurface::new(),
            mpv: None,
            loaded_path: None,
            last_osc_enabled: None,
            last_observed_mpv_fullscreen: None,
            last_mpv_fullscreen: None,
            controls_state: Default::default(),
        }
    }

    /// Returns true if the player is in docked mode
    pub fn is_docked(&self) -> bool {
        self.mode == VideoMode::Docked
    }

    /// Returns true if the player is detached (windowed or fullscreen)
    pub fn is_detached(&self) -> bool {
        self.mode != VideoMode::Docked
    }

    /// Returns true if the player is in fullscreen mode
    pub fn is_fullscreen(&self) -> bool {
        self.mode == VideoMode::Fullscreen
    }

    /// Transition to docked mode
    pub fn dock(&mut self) {
        self.mode = VideoMode::Docked;
        self.forced_size = None;
    }

    /// Transition to detached (windowed) mode
    pub fn detach(&mut self) {
        self.mode = VideoMode::Detached;
    }

    /// Transition to fullscreen mode
    pub fn enter_fullscreen(&mut self) {
        self.mode = VideoMode::Fullscreen;
    }

    /// Transition from fullscreen back to detached
    pub fn exit_fullscreen(&mut self) {
        self.mode = VideoMode::Detached;
        self.restore_frames = 10;
    }

    /// Toggle between docked and detached
    pub fn toggle_detached(&mut self) {
        match self.mode {
            VideoMode::Docked => self.detach(),
            _ => self.dock(),
        }
    }

    /// Toggle between detached and fullscreen
    pub fn toggle_fullscreen(&mut self) {
        match self.mode {
            VideoMode::Fullscreen => self.exit_fullscreen(),
            _ => self.enter_fullscreen(),
        }
    }

    /// Reset the last rect to force window resize on next frame
    pub fn reset_last_rect(&mut self) {
        self.surface.reset_rect();
    }

    /// Temporarily hides the video surface (for popups over the video area)
    pub fn hide_for_overlay(&mut self) {
        self.surface.set_visible(false);
    }

    /// Restores the video surface after closing an overlay
    pub fn restore_from_overlay(&mut self) {
        self.surface.set_visible(self.is_visible);
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
        self.surface.is_initialized()
    }

    pub fn set_visibility(&mut self, visible: bool) {
        if self.is_visible != visible {
            self.is_visible = visible;
            self.surface.set_visible(visible);
        }
    }

    /// Get native HWND for the video surface
    #[cfg(target_os = "windows")]
    pub fn get_hwnd(&self) -> Option<HWND> {
        self.surface.hwnd()
    }

    /// Check if the given HWND matches the video surface
    #[cfg(target_os = "windows")]
    pub fn has_hwnd(&self, hwnd: HWND) -> bool {
        self.surface.has_hwnd(hwnd)
    }

    /// No-op for MPV. Kept for API parity.
    #[cfg(target_os = "windows")]
    pub fn release_focus(&self, main_hwnd: HWND) {
        self.surface.release_focus(main_hwnd);
    }

    /// No-op for MPV. Kept for API parity.
    #[cfg(target_os = "windows")]
    pub fn release_focus_auto(&self) {
        self.surface.release_focus_auto();
    }
}

impl Drop for MpvPreview {
    fn drop(&mut self) {
        self.shutdown();
    }
}
