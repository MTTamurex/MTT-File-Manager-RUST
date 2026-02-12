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
mod playback_state;
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
    last_deinterlace_check: Instant,

    // Performance: Async event handling (Fase 2 optimization)
    event_thread_running: Arc<AtomicBool>,
    event_thread_handle: Option<thread::JoinHandle<()>>,

    // Performance: Caching (polling removed in Fase 2)
    cached_duration: Option<f64>,
    cached_tracks: Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    pending_external_subtitle: Option<PathBuf>,

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

        candidates.into_iter().find(|dir| dir.join("scripts").join("osc.lua").is_file())
    }

    fn create_mpv_instance() -> Result<mpv::Mpv, mpv::Error> {
        if MPV_OSC_POC_ENABLED {
            let config_dir = Self::resolve_mpv_ui_config_dir();
            if config_dir.is_none() {
                eprintln!(
                    "[MpvPreview] MPV UI folder not found (expected mpv_ui/portable_config with scripts/osc.lua)"
                );
            }

            mpv::Mpv::with_initializer(|init| {
                // POC: load MPV UI assets from local folder and keep MPV default input bindings.
                if let Err(e) = init.set_option("load-scripts", true) {
                    eprintln!("[MpvPreview] Failed to set load-scripts=yes: {:?}", e);
                }
                if let Err(e) = init.set_option("osc", false) {
                    eprintln!("[MpvPreview] Failed to set osc=no: {:?}", e);
                }
                if let Err(e) = init.set_option("input-default-bindings", true) {
                    eprintln!(
                        "[MpvPreview] Failed to set input-default-bindings=yes: {:?}",
                        e
                    );
                }
                if let Err(e) = init.set_option("input-vo-keyboard", true) {
                    eprintln!("[MpvPreview] Failed to set input-vo-keyboard=yes: {:?}", e);
                }
                if let Err(e) = init.set_option("input-cursor", true) {
                    eprintln!("[MpvPreview] Failed to set input-cursor=yes: {:?}", e);
                }
                if let Err(e) = init.set_option("cursor-autohide", 1000_i64) {
                    eprintln!("[MpvPreview] Failed to set cursor-autohide=1000: {:?}", e);
                }
                if let Err(e) = init.set_option("script-opts", MPV_OSC_POC_SCRIPT_OPTS) {
                    eprintln!(
                        "[MpvPreview] Failed to set script-opts={} : {:?}",
                        MPV_OSC_POC_SCRIPT_OPTS, e
                    );
                }

                if let Some(dir) = &config_dir {
                    let dir_str = Self::mpv_path_string(dir.as_path());
                    if let Err(e) = init.set_option("config", true) {
                        eprintln!("[MpvPreview] Failed to set config=yes: {:?}", e);
                    }
                    if let Err(e) = init.set_option("config-dir", dir_str.as_str()) {
                        eprintln!(
                            "[MpvPreview] Failed to set config-dir={} : {:?}",
                            dir_str, e
                        );
                    }

                    let osc_script = dir.join("scripts").join("osc.lua");
                    if !osc_script.is_file() {
                        eprintln!(
                            "[MpvPreview] osc.lua not found at {}",
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
                is_playing: false,
                current_time: 0.0,
                duration: 0.0,
                volume: 1.0,
                is_muted: false,
                audio_tracks: Vec::new(),
                subtitle_tracks: Vec::new(),
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
            last_deinterlace_check: Instant::now(),
            event_thread_running: Arc::new(AtomicBool::new(false)),
            event_thread_handle: None,
            cached_duration: None,
            cached_tracks: None,
            pending_external_subtitle: None,
            surface: VideoSurface::new(),
            mpv: None,
            loaded_path: None,
            last_osc_enabled: None,
            last_observed_mpv_fullscreen: None,
            last_mpv_fullscreen: None,
            controls_state: Default::default(),
        }
    }

    pub fn is_native_osc_active(&self) -> bool {
        self.osc_active
    }

    fn desired_osc_enabled(&self) -> bool {
        if !MPV_OSC_POC_ENABLED {
            return false;
        }
        if MPV_OSC_POC_DETACHED_ONLY {
            return self.is_detached();
        }
        true
    }

    fn sync_osc_runtime_state(&mut self, mpv: &mpv::Mpv) {
        let desired_custom_osc_visible = self.desired_osc_enabled();
        if self.last_osc_enabled != Some(desired_custom_osc_visible) {
            // Keep built-in OSC disabled and control only the custom script visibility.
            if let Err(e) = mpv.set_property("osc", false) {
                eprintln!("[MpvPreview] Failed to force osc=no : {:?}", e);
            }

            let visibility_mode = if desired_custom_osc_visible {
                "auto"
            } else {
                "never"
            };
            if let Err(e) = mpv.command("script-message", &["osc-visibility", visibility_mode]) {
                eprintln!(
                    "[MpvPreview] Failed to set custom osc-visibility={} : {:?}",
                    visibility_mode, e
                );
            }
            self.last_osc_enabled = Some(desired_custom_osc_visible);
        }

        let desired_fullscreen = self.is_fullscreen();
        if self.last_mpv_fullscreen != Some(desired_fullscreen) {
            if let Err(e) = mpv.set_property("fullscreen", desired_fullscreen) {
                eprintln!(
                    "[MpvPreview] Failed to set fullscreen={} : {:?}",
                    desired_fullscreen, e
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

    fn sync_fullscreen_from_mpv(&mut self, ui: &egui::Ui, mpv: &mpv::Mpv) {
        let Ok(mpv_fullscreen) = mpv.get_property::<bool>("fullscreen") else {
            return;
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
            self.mode = VideoMode::Detached;
            self.fullscreen_applied = false;
            self.restore_frames = 10;
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

    pub fn update(&mut self, _ui: &mut egui::Ui, _frame: Option<&eframe::Frame>) {
        if !self.show_player {
            self.osc_active = false;
            self.set_visibility(false);
            return;
        }

        let ui = _ui;

        // Reserve space for the video. If forced_size is set (detached mode with control bar), use it.
        let size = if let Some(forced) = self.forced_size {
            forced
        } else if self.is_detached() {
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
            match Self::create_mpv_instance() {
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

                    if MPV_OSC_POC_ENABLED {
                        if let Some(mpv_ref) = &self.mpv {
                            let input_cursor = mpv_ref.get_property::<bool>("input-cursor").ok();
                            let script_count = mpv_ref.get_property::<i64>("script-list/count").ok();
                            eprintln!(
                                "[MpvPreview][OSC-POC] input-cursor={:?}, script-list/count={:?}",
                                input_cursor, script_count
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[MpvPreview] Failed to create MPV: {:?}", e);
                    return;
                }
            }
        }

        self.surface.ensure_main_hwnd(_frame);
        if let Some(m) = &self.mpv {
            self.surface.ensure_child_window(m);
        }

        if let Some(m) = self.mpv.clone() {
            self.sync_fullscreen_from_mpv(ui, &m);
            self.sync_osc_runtime_state(&m);
        }

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
        let is_detached = self.is_detached();
        if is_detached == self.docked_downscale_applied
            || is_detached == self.docked_fps_limit_applied
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

        if self.osc_active {
            if let Some(m) = self.mpv.clone() {
                self.forward_osc_input(ui, rect, &m);
            }
        }

        self.surface.sync_rect(ui, rect);

        // Keep MPV focus while native OSC is active so it can handle input events.
        let should_force_main_focus = !self.osc_active;
        if should_force_main_focus {
            self.surface.ensure_focus_on_main();
        }

        // Context menu removed - controls now in control bar
        // Double-click to toggle fullscreen is handled in preview_panel.rs

        self.set_visibility(self.is_visible);
    }

    fn forward_osc_input(&mut self, ui: &egui::Ui, rect: egui::Rect, mpv: &mpv::Mpv) {
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
