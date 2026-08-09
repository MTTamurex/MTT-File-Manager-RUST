use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
use crate::ui::components::mpv::state::FileLoadGate;
pub use crate::ui::components::mpv::state::{MpvState, PendingSeekState, TrackInfo};
pub use crate::ui::components::mpv::utils::format_time;

const MPV_DOCKED_DEMUXER_MAX_BYTES: i64 = 8_i64 * 1024 * 1024;
const MPV_DOCKED_DEMUXER_MAX_BACK_BYTES: i64 = 2_i64 * 1024 * 1024;
const MPV_DOCKED_AUDIO_VISUALIZATION: &str =
    "[aid1]asplit[ao][a1];[a1]showwaves=s=640x360:mode=cline:rate=24:colors=white,format=pix_fmts=rgb24[vo]";
const MPV_DETACHED_AUDIO_VISUALIZATION: &str =
    "[aid1]asplit[ao][a1];[a1]showwaves=s=1920x1080:mode=cline:rate=30:colors=white,format=pix_fmts=rgb24[vo]";

const MPV_DETACHED_CACHE_SECS: f64 = 20.0;
const MPV_DETACHED_READAHEAD_SECS: f64 = 10.0;
const MPV_DETACHED_DEMUXER_MAX_BYTES: i64 = 96_i64 * 1024 * 1024;
const MPV_DETACHED_DEMUXER_MAX_BACK_BYTES: i64 = 16_i64 * 1024 * 1024;
const MPV_OSC_POC_ENABLED: bool = true;
const MPV_OSC_POC_DETACHED_ONLY: bool = true;
const MPV_OSC_POC_BASE_SCRIPT_OPTS: &str = "osc-scalewindowed=1.8,osc-scalefullscreen=2.8,osc-scaleforcedwindow=1.8,osc-idlescreen=no,osc-showonpause=no";

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
    /// Tracks whether RTX Video features are available on this machine.
    pub is_rtx_supported: bool,
    /// Last mode for which playback properties were applied (`true` = docked).
    last_profile_was_docked: Option<bool>,
    audio_normalizer_enabled: bool,

    // Performance: Async event handling (Fase 2 optimization)
    event_thread_running: Arc<AtomicBool>,
    event_thread_handle: Option<thread::JoinHandle<()>>,

    // Performance: Caching (polling removed in Fase 2)
    cached_duration: Option<f64>,
    cached_tracks: Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    pending_external_subtitle: Option<PathBuf>,

    // PERF: Shared signal for background track querying
    tracks_need_query: Arc<AtomicU64>,
    // PERF: Gate event loop writes during file transitions.
    file_load_gate: Arc<FileLoadGate>,
    // Prevent stale time-pos polls from briefly reverting the seek slider.
    pending_seek: Arc<RwLock<Option<PendingSeekState>>>,
    // PERF: Async sidecar subtitle search receiver
    sidecar_rx: Option<std::sync::mpsc::Receiver<Option<PathBuf>>>,
    // PERF: Track previous interlaced state for change detection
    last_interlaced: Option<bool>,

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
            let osc_script_opts =
                crate::video_player::build_mpv_osc_script_opts(MPV_OSC_POC_BASE_SCRIPT_OPTS);
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

                // Limit libass caches used by the custom OSC/OSD. Older mpv
                // builds may not support these options; keep playback working.
                if let Err(e) = init.set_option(
                    "osd-prune-delay",
                    crate::video_player::MPV_OSD_PRUNE_DELAY_SECS,
                ) {
                    log::warn!("[MpvPreview] Failed to set osd-prune-delay: {:?}", e);
                }
                if let Err(e) =
                    init.set_option("osd-glyph-limit", crate::video_player::MPV_OSD_GLYPH_LIMIT)
                {
                    log::warn!("[MpvPreview] Failed to set osd-glyph-limit: {:?}", e);
                }
                if let Err(e) = init.set_option(
                    "osd-bitmap-max-size",
                    crate::video_player::MPV_OSD_BITMAP_MAX_SIZE_MB,
                ) {
                    log::warn!("[MpvPreview] Failed to set osd-bitmap-max-size: {:?}", e);
                }
                if let Err(e) = init.set_option("osd-shaper", crate::video_player::MPV_OSD_SHAPER) {
                    log::warn!("[MpvPreview] Failed to set osd-shaper: {:?}", e);
                }

                if let Err(e) = init.set_option("script-opts", osc_script_opts.as_str()) {
                    log::warn!(
                        "[MpvPreview] Failed to set script-opts={} : {:?}",
                        osc_script_opts,
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
            is_rtx_supported: false,
            last_profile_was_docked: None,
            audio_normalizer_enabled: false,
            event_thread_running: Arc::new(AtomicBool::new(false)),
            event_thread_handle: None,
            cached_duration: None,
            cached_tracks: None,
            pending_external_subtitle: None,
            tracks_need_query: Arc::new(AtomicU64::new(0)),
            file_load_gate: Arc::new(FileLoadGate::default()),
            pending_seek: Arc::new(RwLock::new(None)),
            sidecar_rx: None,
            last_interlaced: None,
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

    pub(crate) fn set_overlay_rects(
        &mut self,
        overlays: &[crate::ui::video_overlay::VideoOverlay],
        pixels_per_point: f32,
    ) {
        if self.is_docked() {
            self.surface.set_overlay_rects(overlays, pixels_per_point);
        } else {
            self.surface.set_overlay_rects(&[], pixels_per_point);
        }
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

#[cfg(test)]
mod tests {
    use super::{MpvPreview, VideoMode};
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn retarget_resets_file_state_without_replacing_preview_runtime() {
        let mut preview = MpvPreview::new(PathBuf::from("a.mp4"));
        preview.mode = VideoMode::Fullscreen;
        preview.audio_normalizer_enabled = true;
        preview.loaded_path = Some(preview.path.clone());
        if let Ok(mut state) = preview.state.write() {
            state.volume = 0.42;
            state.is_muted = true;
            state.fullscreen = true;
            state.is_playing = true;
            state.current_time = 12.0;
            state.duration = 90.0;
            state.tracks_ready = true;
        }
        let state = Arc::as_ptr(&preview.state);
        let event_loop_signal = Arc::as_ptr(&preview.event_thread_running);
        let generation = Arc::as_ptr(&preview.file_load_gate);

        preview.retarget_for_playback(PathBuf::from("b.mp4"));

        assert_eq!(preview.path, PathBuf::from("b.mp4"));
        assert_eq!(Arc::as_ptr(&preview.state), state);
        assert_eq!(
            Arc::as_ptr(&preview.event_thread_running),
            event_loop_signal
        );
        assert_eq!(Arc::as_ptr(&preview.file_load_gate), generation);
        assert_eq!(preview.mode, VideoMode::Fullscreen);
        assert!(preview.audio_normalizer_enabled);
        let state = preview.state.read().expect("state lock");
        assert_eq!(state.volume, 0.42);
        assert!(state.is_muted);
        assert!(state.fullscreen);
        assert!(!state.is_playing);
        assert_eq!(state.current_time, 0.0);
        assert_eq!(state.duration, 0.0);
        assert!(!state.tracks_ready);
    }
}
