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

// Re-export from sub-modules for backward compatibility
pub use crate::ui::components::mpv::state::{MpvState, TrackInfo};
pub use crate::ui::components::mpv::utils::format_time;
use crate::ui::components::mpv::filters as mpv_filters;
use crate::ui::components::mpv::playback as mpv_playback;
use crate::ui::components::mpv::event_loop as mpv_event_loop;


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

    /// Retorna o estado atual de forma segura, com valor padrão em caso de erro
    pub fn get_state(&self) -> MpvState {
        match self.state.read() {
            Ok(state) => MpvState::clone(&state),
            Err(_) => {
                // Em caso de poison do RwLock, retorna estado padrão
                eprintln!("[MpvPreview] Erro ao ler estado - RwLock poisonado");
                MpvState::default()
            }
        }
    }

    /// Tenta obter o estado com tratamento de erro explícito
    pub fn try_get_state(&self) -> Result<MpvState, String> {
        self.state.read()
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
        match self.state.read() {
            Ok(state) => {
                if state.is_playing {
                    self.pause();
                } else {
                    self.play();
                }
            }
            Err(_) => {
                eprintln!("[MpvPreview] Erro ao toggle play - RwLock poisonado");
                // Tenta pausar como fallback seguro
                self.pause();
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
        // Need to use unsafe transmute or redesign - for now keep direct implementation
        // This is a limitation of the extraction - we need mutable access to state
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
        // CRASH FIX: Check if mpv is available before accessing
        if let Some(m) = &self.mpv {
            // Use try_set_property to avoid blocking on MPV thread
            let _ = m.set_property("mute", muted);
        }
        // Update state regardless of MPV result
        if let Ok(mut state) = self.state.try_write() {
            state.is_muted = muted;
        }
    }

    pub fn toggle_mute(&self) {
        // CRASH FIX: Use try_read to avoid deadlock
        let current_muted = match self.state.try_read() {
            Ok(state) => state.is_muted,
            Err(_) => {
                eprintln!("[MpvPreview] Erro ao ler estado mute - RwLock poisonado ou ocupado");
                // Fallback: assume not muted
                false
            }
        };
        
        // CRASH FIX: Wrap MPV call in catch_unwind to prevent FFI crashes
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.set_muted(!current_muted);
        }));
    }

    pub fn controls_active(&self) -> bool {
        self.last_mouse_activity
            .map(|t| t.elapsed() < Duration::from_secs(3))
            .unwrap_or(false)
    }

    pub fn toggle_audio_normalizer(&mut self) {
        let enabled = !self.audio_normalizer_enabled;
        self.set_audio_normalizer(enabled);
    }

    pub fn is_audio_normalizer_enabled(&self) -> bool {
        self.audio_normalizer_enabled
    }

    fn set_audio_normalizer(&mut self, enabled: bool) {
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

    #[cfg(target_os = "windows")]
    pub fn get_hwnd(&self) -> Option<HWND> {
        self.mpv_hwnd
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

                    // PERF: Low-latency flags to reduce micro-stuttering and improve smoothness
                    // video-sync=display-resample: Sync to display refresh rate (eliminates 24fps→60Hz judder)
                    let _ = m.set_property("video-sync", "display-resample");
                    // interpolation: Enable motion interpolation for smoother playback
                    let _ = m.set_property("interpolation", true);
                    // tscale=oversample: High-quality temporal interpolation
                    let _ = m.set_property("tscale", "oversample");

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
        if let Some(m) = &self.mpv {
            // Check if file is ready by checking if duration is available
            let file_ready = mpv_playback::is_file_ready(m);

            // CACHE: Track list (read once file is ready, then cache until file change)
            if self.cached_tracks.is_none() && file_ready {
                let (audio_tracks, sub_tracks): (Vec<TrackInfo>, Vec<TrackInfo>) = mpv_playback::query_tracks(m);

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

        // Move/resize child window (OPTIMIZED: Only when rect actually changes)
        #[cfg(target_os = "windows")]
        if rect != self.last_rect {
            self.last_rect = rect;

            if let Some(h_video) = self.mpv_hwnd {
                let factor = ui.ctx().pixels_per_point();
                let x = (rect.min.x * factor) as i32;
                let y = (rect.min.y * factor) as i32;
                let w = (rect.width() * factor) as i32;
                let h = (rect.height() * factor) as i32;
                unsafe {
                    // PERF: MoveWindow only called when position/size changes (~95% reduction)
                    let _ = MoveWindow(h_video, x, y, w.max(1), h.max(1), true);
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        if rect != self.last_rect {
            self.last_rect = rect;
        }

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

    /// Applies or removes docked-mode downscale and FPS limiting without restarting playback.
    /// `force_reapply` is used when external changes (e.g., VSR) replace the filter chain.
    fn update_docked_downscale(&mut self, force_reapply: bool) {
        let should_limit = !self.is_detached;
        let Some(m) = &self.mpv else {
            return;
        };

        let current_vf = m.get_property::<String>("vf").unwrap_or_default();
        let has_downscale = current_vf.contains(mpv_filters::DOCKED_DOWNSCALE_MARKER);
        let has_fps_limit = current_vf.contains(mpv_filters::DOCKED_FPS_MARKER);

        if should_limit {
            if force_reapply || !has_downscale || !has_fps_limit {
                // Store current chain to restore on undock (or VSR toggles)
                if self.docked_prev_vf.is_none() {
                    self.docked_prev_vf = Some(current_vf.clone());
                }

                let mut new_vf = current_vf.clone();
                if !has_downscale {
                    new_vf = if new_vf.trim().is_empty() {
                        mpv_filters::DOCKED_DOWNSCALE_FILTER.to_string()
                    } else {
                        format!("{},{}", new_vf, mpv_filters::DOCKED_DOWNSCALE_FILTER)
                    };
                }
                if !has_fps_limit {
                    new_vf = if new_vf.trim().is_empty() {
                        mpv_filters::DOCKED_FPS_FILTER.to_string()
                    } else {
                        format!("{},{}", new_vf, mpv_filters::DOCKED_FPS_FILTER)
                    };
                }
                let _ = m.set_property("vf", new_vf);
            }

            if self.docked_prev_video_sync.is_none() {
                self.docked_prev_video_sync = m.get_property::<String>("video-sync").ok();
            }
            if self.docked_prev_interpolation.is_none() {
                self.docked_prev_interpolation = m.get_property::<bool>("interpolation").ok();
            }
            if self.docked_prev_tscale.is_none() {
                self.docked_prev_tscale = m.get_property::<String>("tscale").ok();
            }

            let _ = m.set_property("video-sync", "audio");
            let _ = m.set_property("interpolation", false);
            let _ = m.set_property("tscale", "linear");

            // Mitigation for slow HDD I/O: increase cache/readahead only in docked preview mode
            if self.docked_prev_cache.is_none() {
                self.docked_prev_cache = m.get_property::<String>("cache").ok();
            }
            if self.docked_prev_cache_secs.is_none() {
                self.docked_prev_cache_secs = m.get_property::<f64>("cache-secs").ok();
            }
            if self.docked_prev_readahead_secs.is_none() {
                self.docked_prev_readahead_secs =
                    m.get_property::<f64>("demuxer-readahead-secs").ok();
            }
            if self.docked_prev_demuxer_max_bytes.is_none() {
                self.docked_prev_demuxer_max_bytes =
                    m.get_property::<i64>("demuxer-max-bytes").ok();
            }
            if self.docked_prev_demuxer_max_back_bytes.is_none() {
                self.docked_prev_demuxer_max_back_bytes =
                    m.get_property::<i64>("demuxer-max-back-bytes").ok();
            }

            let _ = m.set_property("cache", "yes");
            let _ = m.set_property("cache-secs", 20.0f64);
            let _ = m.set_property("demuxer-readahead-secs", 10.0f64);
            let _ = m.set_property("demuxer-max-bytes", 64_i64 * 1024 * 1024);
            let _ = m.set_property("demuxer-max-back-bytes", 16_i64 * 1024 * 1024);

            self.docked_downscale_applied = true;
            self.docked_fps_limit_applied = true;
        } else if self.docked_downscale_applied || self.docked_fps_limit_applied {
            // Restore previous chain and sync behavior (native resolution + normal FPS)
            let restore_vf = self.docked_prev_vf.clone().unwrap_or_default();
            let _ = m.set_property("vf", restore_vf);
            self.docked_prev_vf = None;

            if let Some(prev) = self.docked_prev_video_sync.take() {
                let _ = m.set_property("video-sync", prev);
            }
            if let Some(prev) = self.docked_prev_interpolation.take() {
                let _ = m.set_property("interpolation", prev);
            }
            if let Some(prev) = self.docked_prev_tscale.take() {
                let _ = m.set_property("tscale", prev);
            }

            // Restore cache settings to avoid penalizing SSD and undocked playback
            if let Some(prev) = self.docked_prev_cache.take() {
                let _ = m.set_property("cache", prev);
            }
            if let Some(prev) = self.docked_prev_cache_secs.take() {
                let _ = m.set_property("cache-secs", prev);
            }
            if let Some(prev) = self.docked_prev_readahead_secs.take() {
                let _ = m.set_property("demuxer-readahead-secs", prev);
            }
            if let Some(prev) = self.docked_prev_demuxer_max_bytes.take() {
                let _ = m.set_property("demuxer-max-bytes", prev);
            }
            if let Some(prev) = self.docked_prev_demuxer_max_back_bytes.take() {
                let _ = m.set_property("demuxer-max-back-bytes", prev);
            }

            self.docked_downscale_applied = false;
            self.docked_fps_limit_applied = false;
        }
    }

    fn update_deinterlace_filter(&mut self) {
        let Some(m) = &self.mpv else {
            return;
        };
        let interlaced = match Self::detect_interlaced(m) {
            Some(value) => value,
            None => {
                let _ = m.set_property("deinterlace", "auto");
                return;
            }
        };
        let current_vf = m.get_property::<String>("vf").unwrap_or_default();
        let has_deinterlace = current_vf.contains(mpv_filters::DEINTERLACE_MARKER);

        if interlaced && !has_deinterlace {
            let _ = m.set_property("deinterlace", "yes");
            let new_vf = mpv_filters::append_vf_filter(&current_vf, mpv_filters::DEINTERLACE_FILTER);
            let _ = m.set_property("vf", new_vf);
            self.update_prev_vf_deinterlace(true);
        } else if !interlaced && has_deinterlace {
            let _ = m.set_property("deinterlace", "no");
            let new_vf = mpv_filters::remove_vf_filter(&current_vf, mpv_filters::DEINTERLACE_MARKER);
            let _ = m.set_property("vf", new_vf);
            self.update_prev_vf_deinterlace(false);
        } else if !interlaced {
            let _ = m.set_property("deinterlace", "no");
        }
    }

    fn detect_interlaced(m: &mpv::Mpv) -> Option<bool> {
        if let Ok(value) = m.get_property::<bool>("video-params/interlaced") {
            return Some(value);
        }
        if let Ok(value) = m.get_property::<i64>("video-params/interlaced") {
            return Some(value != 0);
        }
        if let Ok(value) = m.get_property::<String>("video-params/interlaced") {
            let value = value.to_lowercase();
            if value == "yes" || value == "true" || value == "1" {
                return Some(true);
            }
            if value == "no" || value == "false" || value == "0" {
                return Some(false);
            }
        }
        if let Ok(field) = m.get_property::<String>("video-params/field") {
            let field = field.to_lowercase();
            if field == "top" || field == "bottom" || field == "tff" || field == "bff" {
                return Some(true);
            }
            if field == "progressive" {
                return Some(false);
            }
        }
        None
    }

    fn update_prev_vf_deinterlace(&mut self, apply: bool) {
        let Some(prev) = self.docked_prev_vf.clone() else {
            return;
        };
        let updated = if apply {
            if prev.contains(mpv_filters::DEINTERLACE_MARKER) {
                prev
            } else {
                mpv_filters::append_vf_filter(&prev, mpv_filters::DEINTERLACE_FILTER)
            }
        } else if prev.contains(mpv_filters::DEINTERLACE_MARKER) {
            mpv_filters::remove_vf_filter(&prev, mpv_filters::DEINTERLACE_MARKER)
        } else {
            prev
        };
        self.docked_prev_vf = Some(updated);
    }

    /// PERF FASE 2: Starts async polling thread for offloading FFI calls from main thread
    ///
    /// This moves the polling to a background thread, preventing main thread blocking.
    /// Polls at 4 FPS (250ms) but from a separate thread, keeping UI responsive.
    fn start_event_loop_internal(&mut self, mpv: Arc<mpv::Mpv>, ctx: egui::Context) {
        // Don't start if already running
        if self.event_thread_running.load(Ordering::Relaxed) {
            return;
        }

        let handle = mpv_event_loop::start_event_loop(
            mpv,
            self.state.clone(),
            self.event_thread_running.clone(),
            ctx,
        );

        self.event_thread_handle = Some(handle);
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
            // Ensure docked filters are re-applied if needed
            self.update_docked_downscale(true);
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
            // Ensure docked filters are re-applied if needed
            self.update_docked_downscale(true);
            Ok(())
        } else {
            Err("MPV instance not initialized".to_string())
        }
    }
}

impl Drop for MpvPreview {
    fn drop(&mut self) {
        // PERF FASE 2: Gracefully shutdown event loop thread
        mpv_event_loop::stop_event_loop(
            self.event_thread_running.clone(),
            self.event_thread_handle.take(),
        );

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.mpv_hwnd.take() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}
