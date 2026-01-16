use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use std::io::{Read, Write, BufReader, BufRead};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use eframe::egui;
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOW, FindWindowExW};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

/// Shared state for video playback (updated via IPC from JavaScript)
#[derive(Clone, Default)]
pub struct VideoState {
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_muted: bool,
}

pub struct WebviewPreview {
    pub path: PathBuf,
    webview: Option<WebView>,
    last_rect: egui::Rect,
    server_port: Option<u16>,
    _server_shutdown: Option<mpsc::Sender<()>>,
    // Async Init
    init_rx: Option<mpsc::Receiver<(u16, mpsc::Sender<()>, VideoCodecInfo)>>,
    
    // Player state
    pub show_player: bool,     // false = show thumbnail, true = show video
    pub play_on_init: bool,    // if true, play as soon as webview is ready
    pub state: Arc<RwLock<VideoState>>,
    pub is_visible: bool,      // Track intended visibility state
    pub is_detached: bool,     // Track if player is detached into a floating window
    pub is_maximized: bool,    // Track if detached window is maximized
    pub fullscreen_applied: bool, // Whether fullscreen command was applied
    pub prev_app_maximized: bool, // App maximized state before fullscreen
    pub restore_needed: bool,  // Signal to restore window size on next frame
    pub last_window_rect: Option<egui::Rect>, // Track window size before maximize
    pub forced_size: Option<egui::Vec2>, // Explicit size override to prevent resize loops
    pub last_mouse_activity: Arc<Mutex<Option<Instant>>>, // Last mouse activity from WebView
    pub mouse_over: Arc<Mutex<bool>>, // Whether mouse is inside WebView
    
    #[cfg(target_os = "windows")]
    webview_hwnd: Arc<Mutex<Option<HWND>>>,
}

impl WebviewPreview {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            webview: None,
            last_rect: egui::Rect::NAN,
            server_port: None,
            _server_shutdown: None,
            init_rx: None,
            show_player: false,
            play_on_init: false,
            state: Arc::new(RwLock::new(VideoState {
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
            last_mouse_activity: Arc::new(Mutex::new(None)),
            mouse_over: Arc::new(Mutex::new(false)),
            #[cfg(target_os = "windows")]
            webview_hwnd: Arc::new(Mutex::new(None)),
        }
    }

    /// Get current playback state
    pub fn get_state(&self) -> VideoState {
        self.state.read().unwrap().clone()
    }

    /// Start video playback
    pub fn play(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script("document.getElementById('player').play()");
        }
    }

    /// Pause video playback
    pub fn pause(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script("document.getElementById('player').pause()");
        }
    }

    /// Toggle play/pause
    pub fn toggle_play(&mut self) {
        if self.webview.is_none() {
            self.show_player = true;
            self.play_on_init = true;
        } else {
            let is_playing = self.state.read().unwrap().is_playing;
            if is_playing {
                self.pause();
            } else {
                self.play();
            }
        }
    }

    /// Seek to specific time in seconds
    pub fn seek(&self, time: f64) {
        if let Some(webview) = &self.webview {
            // Try smart seek first (for transcoded videos), fallback to native
            let _ = webview.evaluate_script(&format!(
                "if (typeof seekToPosition === 'function') {{ seekToPosition({}); }} else {{ document.getElementById('player').currentTime = {}; }}", 
                time, time
            ));
        }
    }

    /// Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script(&format!(
                "document.getElementById('player').volume = {}; document.getElementById('player').muted = false;", volume.clamp(0.0, 1.0)
            ));
        }
        if let Ok(mut state) = self.state.write() {
            state.volume = volume;
            state.is_muted = false;
        }
    }

    /// Set muted state
    pub fn set_muted(&self, muted: bool) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script(&format!(
                "document.getElementById('player').muted = {}", muted
            ));
        }
        if let Ok(mut state) = self.state.write() {
            state.is_muted = muted;
        }
    }

    /// Toggle mute
    pub fn toggle_mute(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script(
                "var v = document.getElementById('player'); v.muted = !v.muted;"
            );
        }
    }

    /// Whether to show controls based on recent mouse activity inside WebView
    pub fn controls_active(&self) -> bool {
        // Only check if there was recent mouse MOVEMENT (not just presence)
        self.last_mouse_activity
            .lock()
            .ok()
            .and_then(|v| *v)
            .map(|t| t.elapsed() < Duration::from_secs(3)) // 3 seconds after last movement
            .unwrap_or(false)
    }
    
    /// Reset mouse activity (call when controls become visible to start hide timer)
    pub fn touch_mouse_activity(&self) {
        if let Ok(mut last) = self.last_mouse_activity.lock() {
            *last = Some(Instant::now());
        }
    }
    
    /// Release keyboard focus from WebView2 back to the main window
    /// Call this when user clicks outside the video player area
    #[cfg(target_os = "windows")]
    pub fn release_focus(&self, main_hwnd: HWND) {
        unsafe {
            // Set focus to the main window to release WebView2's keyboard capture
            let _ = SetFocus(main_hwnd);
        }
    }
    
    /// Release keyboard focus automatically by getting foreground window
    /// This is simpler to use - just call it when clicking outside player
    #[cfg(target_os = "windows")]
    pub fn release_focus_auto(&self) {
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        unsafe {
            let hwnd = GetForegroundWindow();
            if !hwnd.is_invalid() {
                let _ = SetFocus(hwnd);
            }
        }
    }
    
    /// Check if this webview has the given HWND as its child
    #[cfg(target_os = "windows")]
    pub fn has_hwnd(&self, hwnd: HWND) -> bool {
        if let Ok(guard) = self.webview_hwnd.lock() {
            if let Some(wv_hwnd) = *guard {
                return wv_hwnd == hwnd;
            }
        }
        false
    }

    fn finalize_init(&mut self, _ctx: &egui::Context, _ui: &egui::Ui, window: &dyn raw_window_handle::HasWindowHandle, port: u16, shutdown_tx: mpsc::Sender<()>, codec_info: VideoCodecInfo) {
        self.server_port = Some(port);
        self._server_shutdown = Some(shutdown_tx);
        
        let is_compatible = is_codec_webview_compatible(&codec_info);
        let duration = codec_info.duration.unwrap_or(0.0);
        
        // Build URL with mode flag - server will route based on this, NO per-request probe
        let mode = if is_compatible { "direct" } else { "transcode" };
        let video_url = format!("http://127.0.0.1:{}/video.mp4?mode={}", port, mode);
        println!("[WebviewPreview] Video URL: {} (duration: {:.1}s)", video_url, duration);
        
        let needs_transcoding = !is_compatible;
        
        // Clone state for IPC handler
        let state_clone = self.state.clone();
        let mouse_activity = self.last_mouse_activity.clone();
        let mouse_over = self.mouse_over.clone();
        
        // Build HTML - different behavior for transcoded vs native files
        let html_content = if needs_transcoding && duration > 0.0 {
            // Smart player for transcoded videos with seek support
            format!(r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ 
            background: #1a1a1a; 
            display: flex; 
            align-items: center; 
            justify-content: center; 
            width: 100vw;
            height: 100vh;
            overflow: hidden; 
        }}
        video {{ 
            width: 100%; 
            height: 100%; 
            object-fit: contain; 
        }}
    </style>
</head>
<body oncontextmenu="return false;">
    <video id="player" autoplay oncontextmenu="return false;"></video>
    <script>
        const video = document.getElementById('player');
        const baseUrl = '{}';
        const knownDuration = {};
        let currentSeekOffset = 0;
        let isReloading = false;
        
        console.log("Smart transcoding player loaded. Duration:", knownDuration);
        
        // Load video at specific offset
        function loadVideoAt(seekSeconds) {{
            isReloading = true;
            currentSeekOffset = seekSeconds;
            // baseUrl already has ?mode=transcode, so use & for additional params
            const url = seekSeconds > 0 ? baseUrl + '&seek=' + seekSeconds.toFixed(2) : baseUrl;
            console.log("Loading:", url);
            video.src = url;
            video.load();
            video.play().catch(e => console.log("Autoplay blocked:", e));
            isReloading = false;
        }}
        
        // Initial load
        loadVideoAt(0);
        
        // Intercept user seeking on progress bar
        let pendingSeek = null;
        video.addEventListener('seeking', () => {{
            if (!isReloading && video.seeking) {{
                // User initiated seek - capture target time
                pendingSeek = video.currentTime + currentSeekOffset;
            }}
        }});
        video.addEventListener('seeked', () => {{
            if (pendingSeek !== null && !isReloading) {{
                const targetTime = pendingSeek;
                pendingSeek = null;
                // Only reload if seeking more than 5 seconds from current offset start
                if (Math.abs(targetTime - currentSeekOffset) > 5) {{
                    console.log("User seek to:", targetTime);
                    loadVideoAt(targetTime);
                }}
            }}
        }});
        
        // Report state to Rust (Adaptive Polling)
        const reportState = () => {{
            try {{
                // Real time = stream time + offset
                const realTime = video.currentTime + currentSeekOffset;
                const state = JSON.stringify({{
                    type: 'state',
                    playing: !video.paused,
                    currentTime: realTime,
                    duration: knownDuration,
                    volume: video.volume,
                    muted: video.muted
                }});
                window.ipc.postMessage(state);
            }} catch(e) {{
                console.error("IPC error:", e);
            }}
            
            // Throttle: 100ms when playing, 500ms when paused
            setTimeout(reportState, video.paused ? 500 : 100);
        }};
        reportState();

        // Mouse activity reporting for autohide controls
        let lastMouseSent = 0;
        const sendMouse = () => {{
            const now = Date.now();
            if (now - lastMouseSent > 120) {{
                lastMouseSent = now;
                window.ipc.postMessage(JSON.stringify({{ type: 'mouse_move' }}));
            }}
        }};
        document.addEventListener('mousemove', sendMouse);
        document.addEventListener('mouseenter', sendMouse);
        document.addEventListener('mouseleave', () => {{
            window.ipc.postMessage(JSON.stringify({{ type: 'mouse_leave' }}));
        }});
        
        // Report events
        video.addEventListener('play', () => {{
            console.log("Video playing");
            window.ipc.postMessage(JSON.stringify({{ type: 'play' }}));
        }});
        video.addEventListener('pause', () => {{
            console.log("Video paused");
            window.ipc.postMessage(JSON.stringify({{ type: 'pause' }}));
        }});
        video.addEventListener('ended', () => {{
            console.log("Video ended");
            window.ipc.postMessage(JSON.stringify({{ type: 'ended' }}));
        }});
        
        // Listen for seek commands from Rust
        window.seekToPosition = function(seconds) {{
            console.log("Seek requested to:", seconds);
            if (Math.abs(seconds - (video.currentTime + currentSeekOffset)) > 2) {{
                // Big seek - reload with new offset
                loadVideoAt(seconds);
            }}
        }};
    </script>
</body>
</html>"#, video_url, duration)
        } else {
            // Standard player for native formats (or unknown duration)
            format!(r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ 
            background: #1a1a1a; 
            display: flex; 
            align-items: center; 
            justify-content: center; 
            width: 100vw;
            height: 100vh;
            overflow: hidden; 
        }}
        video {{ 
            width: 100%; 
            height: 100%; 
            object-fit: contain; 
        }}
    </style>
</head>
<body oncontextmenu="return false;">
    <video id="player" autoplay oncontextmenu="return false;">
        <source src="{}" type="video/mp4">
    </video>
    <script>
        const video = document.getElementById('player');
        
        console.log("Video player script loaded");

        // Report state to Rust (Adaptive Polling)
        const reportState = () => {{
            try {{
                const state = JSON.stringify({{
                    type: 'state',
                    playing: !video.paused,
                    currentTime: video.currentTime,
                    duration: video.duration || 0,
                    volume: video.volume,
                    muted: video.muted
                }});
                window.ipc.postMessage(state);
            }} catch(e) {{
                console.error("IPC error:", e);
            }}
            
            // Throttle: 100ms when playing, 500ms when paused
            setTimeout(reportState, video.paused ? 500 : 100);
        }};
        reportState();

        // Mouse activity reporting for autohide controls (only on actual movement)
        let lastX = 0, lastY = 0, lastSent = 0;
        document.addEventListener('mousemove', (e) => {{
            const dx = Math.abs(e.clientX - lastX);
            const dy = Math.abs(e.clientY - lastY);
            const now = Date.now();
            // Only report if mouse actually moved significantly (>3px) and throttle to 200ms
            if ((dx > 3 || dy > 3) && (now - lastSent > 200)) {{
                lastX = e.clientX;
                lastY = e.clientY;
                lastSent = now;
                window.ipc.postMessage(JSON.stringify({{ type: 'mouse_move' }}));
            }}
        }});
        
        // Report events
        video.addEventListener('play', () => {{
            console.log("Video playing");
            window.ipc.postMessage(JSON.stringify({{ type: 'play' }}));
        }});
        video.addEventListener('pause', () => {{
            console.log("Video paused");
            window.ipc.postMessage(JSON.stringify({{ type: 'pause' }}));
        }});
        video.addEventListener('ended', () => {{
            console.log("Video ended");
            window.ipc.postMessage(JSON.stringify({{ type: 'ended' }}));
        }});
    </script>
</body>
</html>"#, video_url)
        };

        if let Ok(handle) = window.window_handle() {
            let webview = WebViewBuilder::new_as_child(&handle)
                .with_html(html_content)
                .with_ipc_handler(move |msg| {
                    let body = msg.body();
                    // println!("[WebviewPreview] IPC Message: {}", body);
                    
                    // Parse IPC message from JavaScript
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(msg_type) = json.get("type").and_then(|v| v.as_str()) {
                            match msg_type {
                                "state" => {
                                    if let Ok(mut state) = state_clone.write() {
                                        state.is_playing = json.get("playing")
                                            .and_then(|v: &serde_json::Value| v.as_bool()).unwrap_or(false);
                                        state.current_time = json.get("currentTime")
                                            .and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(0.0);
                                        state.duration = json.get("duration")
                                            .and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(0.0);
                                        state.volume = json.get("volume")
                                            .and_then(|v: &serde_json::Value| v.as_f64()).unwrap_or(1.0) as f32;
                                        state.is_muted = json.get("muted")
                                            .and_then(|v: &serde_json::Value| v.as_bool()).unwrap_or(false);
                                    }
                                }
                                "play" => {
                                    if let Ok(mut state) = state_clone.write() {
                                        state.is_playing = true;
                                    }
                                }
                                "pause" => {
                                    if let Ok(mut state) = state_clone.write() {
                                        state.is_playing = false;
                                    }
                                }
                                "mouse_move" => {
                                    if let Ok(mut over) = mouse_over.lock() {
                                        *over = true;
                                    }
                                    if let Ok(mut last) = mouse_activity.lock() {
                                        *last = Some(Instant::now());
                                    }
                                }
                                "mouse_leave" => {
                                    if let Ok(mut over) = mouse_over.lock() {
                                        *over = false;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                })
                .with_devtools(true) // Enable devtools for easier debugging
                .with_background_color((26, 26, 26, 255))
                .build();

            match webview {
                Ok(wv) => {
                    println!("[WebviewPreview] WebView created successfully");
                    
                    #[cfg(target_os = "windows")]
                    {
                        // Get the child HWND created by wry.
                        // Since we created it 'as_child' of 'handle', it will be a child window.
                        unsafe {
                            if let Ok(parent_handle) = window.window_handle() {
                                if let raw_window_handle::RawWindowHandle::Win32(wh) = parent_handle.as_raw() {
                                    let parent_hwnd = HWND(wh.hwnd.get() as _);
                                    // Find child by trying to find any window inside parent.
                                    // WebView2 creates a child window.
                                    if let Ok(child) = FindWindowExW(parent_hwnd, None, None, None) {
                                        if !child.is_invalid() {
                                            println!("[WebviewPreview] Found Child HWND: {:?}", child);
                                            if let Ok(mut h) = self.webview_hwnd.lock() {
                                                *h = Some(child);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    self.webview = Some(wv);
                    
                    if self.play_on_init {
                        self.play();
                        self.play_on_init = false;
                    }
                    
                    // Update state to playing since autoplay is on
                    if let Ok(mut state) = self.state.write() {
                        state.is_playing = true;
                    }
                },
                Err(e) => eprintln!("[WebviewPreview] Failed to create WebView: {}", e),
            }
        }
    }

    pub fn update(&mut self, ui: &mut egui::Ui, frame: Option<&eframe::Frame>) {
        // Only render if show_player is true
        if !self.show_player {
            return;
        }
        
        // Reserve space for the video. If forced_size is set (detached mode with control bar), use it.
        let size = if let Some(forced) = self.forced_size {
            forced
        } else if self.is_detached {
            // Detached window fills available space
            ui.available_size()
        } else {
            // Attached preview: keep aspect-friendly height
            let available = ui.available_size();
            let preview_height = (available.x * 0.6).min(300.0);
            egui::vec2(available.x, preview_height)
        };
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

        // Async Lazy Init
        if self.webview.is_none() {
            // Check if initialization is in progress
            if let Some(rx) = &self.init_rx {
                // Poll for completion
                match rx.try_recv() {
                    Ok((port, shutdown_tx, codec_info)) => {
                        // Init complete, create webview
                        if let Some(frame) = frame {
                            use raw_window_handle::HasWindowHandle;
                            if let Ok(handle) = frame.window_handle() {
                                self.finalize_init(ui.ctx(), ui, &handle, port, shutdown_tx, codec_info);
                                self.init_rx = None; // Done
                            }
                        }
                    },
                    Err(mpsc::TryRecvError::Empty) => {
                        // Still loading
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.spinner();
                                ui.label("Carregando player...");
                            });
                        });
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
                        return; // Don't layout webview yet
                    },
                    Err(mpsc::TryRecvError::Disconnected) => {
                         // Failed
                         ui.label("Erro ao iniciar servidor de vídeo.");
                         self.init_rx = None;
                         return;
                    }
                }
            } else {
                // First frame: Start background initialization
                let path_clone = self.path.clone();
                let (tx, rx) = mpsc::channel();
                self.init_rx = Some(rx);
                
                thread::spawn(move || {
                     // 1. Start video server
                     if let Some((port, shutdown_tx)) = spawn_video_server(path_clone.clone()) {
                         // 2. Probe codecs
                         let info = probe_video_codecs_internal(&path_clone);
                         let _ = tx.send((port, shutdown_tx, info));
                     }
                });
                
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                    });
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
                return;
            }
        }

        // Sync Bounds
        if let Some(webview) = &self.webview {
            if rect != self.last_rect {
                let pixels_per_point = ui.ctx().pixels_per_point();
                let physical_rect = rect_to_physical(rect, pixels_per_point);
                
                let _ = webview.set_bounds(wry::Rect {
                    position: wry::dpi::PhysicalPosition::new(physical_rect.min.x as i32, physical_rect.min.y as i32).into(),
                    size: wry::dpi::PhysicalSize::new(physical_rect.width() as u32, physical_rect.height() as u32).into(),
                });
                
                self.last_rect = rect;
            }
            
            // Respect the is_visible flag - if false, we keep it hidden
            if !self.is_visible {
                let _ = webview.set_visible(false);
            } else if !ui.is_rect_visible(rect) {
                let _ = webview.set_visible(false);
            } else {
                let _ = webview.set_visible(true);
            }
        }
        
        // Request repaint to keep state updated (adaptive throttle)
        let is_playing = if let Ok(s) = self.state.read() { s.is_playing } else { false };
        let delay = if is_playing && self.is_visible { 120 } else { 600 };
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(delay));
    }
    
    pub fn try_init(&mut self, _window: &dyn raw_window_handle::HasWindowHandle, _ctx: &egui::Context, _ui: &egui::Ui) {
        if self.webview.is_none() {
        // try_init is deprecated for async flow, doing nothing to avoid double-init risks
        // self.init_webview(ctx, ui, window); 
        // The update loop will handle it.
        }
    }
    
    /// Check if webview is initialized
    pub fn is_initialized(&self) -> bool {
        self.webview.is_some()
    }

    /// Set WebView visibility (show/hide).
    /// Used for tab isolation - hides video when not on owner tab.
    /// Audio continues when hidden.
    pub fn set_visibility(&mut self, visible: bool) {
        self.is_visible = visible;
        if let Some(ref wv) = self.webview {
            // 1. Try wry's built-in visibility logic
            let _ = wv.set_visible(visible);

            // 2. FORCE visibility using native Windows API for the HWND.
            // This is essential to prevent visual leaks between tabs and 
            // ensure the WebView doesn't intercept mouse input in non-owner tabs.
            #[cfg(target_os = "windows")]
            {
                if let Ok(hwnd_opt) = self.webview_hwnd.lock() {
                    if let Some(hwnd) = *hwnd_opt {
                        unsafe {
                            let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
                        }
                    }
                }
            }
        }
    }
}

impl Drop for WebviewPreview {
    fn drop(&mut self) {
        // Signal server shutdown
        if let Some(tx) = &self._server_shutdown {
            let _ = tx.send(());
        }
        
        // Hide WebView window immediately to prevent ghost windows
        if let Some(webview) = &self.webview {
            let _ = webview.set_visible(false);
            
            #[cfg(target_os = "windows")]
            if let Ok(hwnd_opt) = self.webview_hwnd.lock() {
                if let Some(hwnd) = *hwnd_opt {
                    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                    unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
                }
            }
        }
    }
}

/// Spawn video server on a background thread
fn spawn_video_server(path: PathBuf) -> Option<(u16, mpsc::Sender<()>)> {
    use crate::infrastructure::media::ffmpeg_session::FfmpegSession;
    
    let video_path = dunce::canonicalize(&path).unwrap_or(path);
        
    // Try to find an available port
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[VideoServer] Failed to bind: {}", e);
            return None;
        }
    };
    
    let port = listener.local_addr().ok()?.port();
    println!("[VideoServer] Started on port {}", port);
    
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    
    // Create SHARED FFmpeg session - only ONE ffmpeg per video server
    let ffmpeg_session = FfmpegSession::new();
    
    // Spawn server thread
    thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        
        loop {
            // Check for shutdown
            if shutdown_rx.try_recv().is_ok() {
                println!("[VideoServer] Shutting down");
                // Kill any active FFmpeg when server shuts down
                ffmpeg_session.kill("server shutdown");
                break;
            }
            
            // Accept connections
            match listener.accept() {
                Ok((stream, _)) => {
                    let path = video_path.clone();
                    let session = ffmpeg_session.clone(); // Clone Arc, not the session
                    thread::spawn(move || {
                        handle_video_request(stream, &path, session);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    eprintln!("[VideoServer] Accept error: {}", e);
                }
            }
        }
    });
    
    Some((port, shutdown_tx))
}

// ========================================
// SMART STREAMING SERVER
// ========================================

/// Codec information extracted from video file
#[derive(Debug, Clone)]
struct VideoCodecInfo {
    video_codec: Option<String>,
    audio_codec: Option<String>,
    duration: Option<f64>,
}

/// Internal: Probe video file to get codec information using ffprobe
fn probe_video_codecs_internal(video_path: &PathBuf) -> VideoCodecInfo {
    let file_path = video_path.to_string_lossy().to_string();
    
    // Probe video codec
    let video_codec = {
        let mut cmd = Command::new("ffprobe");
        cmd.args(&[
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_name",
            "-of", "default=noprint_wrappers=1:nokey=1",
            &file_path
        ]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        
        cmd.output().ok().and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
            if s.is_empty() { None } else { Some(s) }
        })
    };
    
    // Probe audio codec
    let audio_codec = {
        let mut cmd = Command::new("ffprobe");
        cmd.args(&[
            "-v", "error",
            "-select_streams", "a:0",
            "-show_entries", "stream=codec_name",
            "-of", "default=noprint_wrappers=1:nokey=1",
            &file_path
        ]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        
        cmd.output().ok().and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_lowercase();
            if s.is_empty() { None } else { Some(s) }
        })
    };
    
    // Probe duration
    let duration = {
        let mut cmd = Command::new("ffprobe");
        cmd.args(&[
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            &file_path
        ]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        
        cmd.output().ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()
        })
    };
    
    println!("[SmartServer] Probed: video={:?}, audio={:?}, duration={:?}", 
        video_codec, audio_codec, duration);
    
    VideoCodecInfo { video_codec, audio_codec, duration }
}

/// Check if codecs are compatible with WebView2 (Chromium/Edge)
fn is_codec_webview_compatible(info: &VideoCodecInfo) -> bool {
    // Whitelist of WebView2-compatible codecs
    const VIDEO_WHITELIST: &[&str] = &["h264", "vp8", "vp9", "av1", "theora"];
    const AUDIO_WHITELIST: &[&str] = &["aac", "mp3", "opus", "vorbis", "flac", "pcm_s16le", "pcm_f32le"];
    
    let video_ok = info.video_codec.as_ref()
        .map(|c| VIDEO_WHITELIST.iter().any(|w| c.contains(w)))
        .unwrap_or(true); // No video = ok
    
    let audio_ok = info.audio_codec.as_ref()
        .map(|c| AUDIO_WHITELIST.iter().any(|w| c.contains(w)))
        .unwrap_or(true); // No audio = ok
    
    let compatible = video_ok && audio_ok;
    println!("[SmartServer] Codec compatible: {} (video_ok={}, audio_ok={})", 
        compatible, video_ok, audio_ok);
    
    compatible
}

// FfmpegGuard removed - replaced by centralized FfmpegSession in infrastructure::media::ffmpeg_session

/// Parse seek parameter from URL query string
fn parse_seek_param(request_line: &str) -> Option<f64> {
    if let Some(query_start) = request_line.find('?') {
        let query = &request_line[query_start..];
        for param in query.split('&') {
            let param = param.trim_start_matches('?');
            if let Some(value) = param.strip_prefix("seek=") {
                let value = value.split_whitespace().next().unwrap_or(value);
                if let Ok(parsed) = value.parse::<f64>() {
                    println!("[SmartServer] Seek parameter: {} seconds", parsed);
                    return Some(parsed);
                }
            }
        }
    }
    None
}

/// ROUTE A: Direct Play - serve file with Range Request support
fn handle_direct_play(mut stream: TcpStream, video_path: &PathBuf, range_header: Option<String>) {
    use std::io::Seek;
    use std::fs::File;
    
    let mut file = match File::open(video_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[DirectPlay] Failed to open file: {}", e);
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            return;
        }
    };
    
    let total_size = match file.metadata() {
        Ok(m) => m.len() as usize,
        Err(_) => {
            let _ = stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n");
            return;
        }
    };
    
    // Determine MIME type
    let path_str = video_path.to_string_lossy();
    let mime = if path_str.ends_with(".mp4") { "video/mp4" }
        else if path_str.ends_with(".webm") { "video/webm" }
        else if path_str.ends_with(".mkv") { "video/x-matroska" }
        else if path_str.ends_with(".mov") { "video/quicktime" }
        else { "video/mp4" };
    
    if let Some(range) = range_header {
        // 206 Partial Content
        if let Some(range_spec) = range.strip_prefix("Range: bytes=")
            .or_else(|| range.strip_prefix("range: bytes=")) 
        {
            let parts: Vec<&str> = range_spec.trim().split('-').collect();
            let start: usize = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
            let max_chunk = 2 * 1024 * 1024; // 2MB chunks
            let requested_end: usize = parts.get(1)
                .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
                .unwrap_or((start + max_chunk).min(total_size - 1));
            let end = requested_end.min(total_size - 1);
            let chunk_size = end - start + 1;
            
            if file.seek(std::io::SeekFrom::Start(start as u64)).is_err() {
                let _ = stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n");
                return;
            }
            
            let mut buffer = vec![0u8; chunk_size];
            let bytes_read = file.read(&mut buffer).unwrap_or(0);
            buffer.truncate(bytes_read);
            
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\n\
                Content-Type: {}\r\n\
                Content-Length: {}\r\n\
                Content-Range: bytes {}-{}/{}\r\n\
                Accept-Ranges: bytes\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: keep-alive\r\n\
                \r\n",
                mime, buffer.len(), start, start + buffer.len().saturating_sub(1), total_size
            );
            
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&buffer);
        }
    } else {
        // 200 OK with full Content-Length (supports Range for subsequent requests)
        let initial_chunk = (1024 * 1024).min(total_size);
        let mut buffer = vec![0u8; initial_chunk];
        let bytes_read = file.read(&mut buffer).unwrap_or(0);
        buffer.truncate(bytes_read);
        
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
            Content-Type: {}\r\n\
            Content-Length: {}\r\n\
            Accept-Ranges: bytes\r\n\
            Access-Control-Allow-Origin: *\r\n\
            \r\n",
            mime, total_size
        );
        
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(&buffer);
    }
}

/// ROUTE B: Transcoding - transcode with FFmpeg and stream
/// Uses centralized FfmpegSession to ensure only ONE FFmpeg per session.
fn handle_transcoding(
    mut stream: TcpStream, 
    video_path: &PathBuf, 
    seek_seconds: Option<f64>,
    duration: Option<f64>,
    session: crate::infrastructure::media::ffmpeg_session::FfmpegSession
) {
    let file_path = video_path.to_string_lossy().to_string();
    
    // CRITICAL: Kill existing FFmpeg BEFORE spawning new one
    // This prevents process accumulation on seek
    let reason = if seek_seconds.is_some() { "seek request" } else { "new transcode" };
    session.kill(reason);
    
    if let Some(seek) = seek_seconds {
        println!("[Transcoding] Starting FFmpeg for: {} (seek: {:.2}s)", file_path, seek);
    } else {
        println!("[Transcoding] Starting FFmpeg for: {}", file_path);
    }

    // Try each profile until one works
    let profiles = crate::infrastructure::media::hardware_acceleration::get_prioritized_profiles();
    
    let mut ffmpeg_stdout: Option<std::process::ChildStdout> = None;
    let mut first_chunk_data: Vec<u8> = Vec::new();
    
    for (attempt, profile) in profiles.iter().enumerate() {
        println!("[Transcoding] Attempt {} with profile: {}", attempt + 1, profile.name);
        
        let args = crate::infrastructure::media::hardware_acceleration::build_transcode_args(
            profile, 
            &file_path, 
            seek_seconds
        );
        
        // Spawn via centralized session
        let stdout = match session.spawn(args) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[Transcoding] Failed to spawn FFmpeg: {}", e);
                continue;
            }
        };
        
        // Validate: read first chunk to verify encoder works
        let mut temp_stdout = stdout;
        let mut temp_buffer = vec![0u8; 64 * 1024];
        
        match temp_stdout.read(&mut temp_buffer) {
            Ok(n) if n > 0 => {
                // Success! Encoder is producing data
                temp_buffer.truncate(n);
                first_chunk_data = temp_buffer;
                ffmpeg_stdout = Some(temp_stdout);
                println!("[Transcoding] Profile {} started successfully (PID: {:?})", profile.name, session.pid());
                break;
            }
            Ok(_) => {
                // EOF immediately = encoder failed
                if let Some(stderr) = session.read_stderr() {
                    if !stderr.is_empty() {
                        println!("[Transcoding] Profile {} FAILED - FFmpeg stderr:\n{}", profile.name, stderr);
                    } else {
                        println!("[Transcoding] Profile {} returned EOF immediately.", profile.name);
                    }
                } else {
                    println!("[Transcoding] Profile {} returned EOF immediately.", profile.name);
                }
                session.kill("profile failed");
                continue;
            }
            Err(e) => {
                println!("[Transcoding] Profile {} read error: {}", profile.name, e);
                session.kill("read error");
                continue;
            }
        }
    }
    
    // Check if we have a working encoder
    let mut ffmpeg_stdout = match ffmpeg_stdout {
        Some(s) => s,
        None => {
            eprintln!("[Transcoding] All profiles failed.");
            let _ = stream.write_all(b"HTTP/1.1 500 Processing Error\r\n\r\n");
            return;
        }
    };

    // HTTP headers
    let mut headers = String::from(
        "HTTP/1.1 200 OK\r\n\
        Content-Type: video/mp4\r\n\
        Transfer-Encoding: chunked\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Cache-Control: no-cache\r\n\
        Connection: close\r\n"
    );
    if let Some(dur) = duration {
        headers.push_str(&format!("X-Video-Duration: {:.2}\r\n", dur));
    }
    headers.push_str("\r\n");
    
    if stream.write_all(headers.as_bytes()).is_err() {
        session.kill("client disconnected before headers");
        return;
    }
    
    // Send first chunk (from validation)
    if !first_chunk_data.is_empty() {
        let n = first_chunk_data.len();
        let chunk_header = format!("{:x}\r\n", n);
        if !write_with_backpressure(&mut stream, chunk_header.as_bytes()) { 
            session.kill("client disconnected");
            return; 
        }
        if !write_with_backpressure(&mut stream, &first_chunk_data) { 
            session.kill("client disconnected");
            return; 
        }
        if !write_with_backpressure(&mut stream, b"\r\n") { 
            session.kill("client disconnected");
            return; 
        }
    }
    
    // Stream with chunked encoding
    let mut buffer = [0u8; 64 * 1024];
    let mut total_bytes: u64 = first_chunk_data.len() as u64;
    let mut last_logged_mb: u64 = 0;
    
    loop {
        match ffmpeg_stdout.read(&mut buffer) {
            Ok(0) => {
                // EOF - FFmpeg finished naturally
                let _ = stream.write_all(b"0\r\n\r\n");
                let _ = stream.flush();
                println!("[Transcoding] Complete: {:.1} MB sent", total_bytes as f64 / (1024.0 * 1024.0));
                break;
            }
            Ok(n) => {
                let chunk_header = format!("{:x}\r\n", n);
                
                if !write_with_backpressure(&mut stream, chunk_header.as_bytes()) {
                    println!("[Transcoding] Client disconnected ({:.1} MB sent)", total_bytes as f64 / (1024.0 * 1024.0));
                    session.kill("client disconnected");
                    break;
                }
                
                if !write_with_backpressure(&mut stream, &buffer[..n]) {
                    println!("[Transcoding] Client disconnected ({:.1} MB sent)", total_bytes as f64 / (1024.0 * 1024.0));
                    session.kill("client disconnected");
                    break;
                }
                
                if !write_with_backpressure(&mut stream, b"\r\n") {
                    println!("[Transcoding] Client disconnected ({:.1} MB sent)", total_bytes as f64 / (1024.0 * 1024.0));
                    session.kill("client disconnected");
                    break;
                }
                
                total_bytes += n as u64;
                
                // Log progress every 5MB
                let current_mb = total_bytes / (5 * 1024 * 1024);
                if current_mb > last_logged_mb {
                    last_logged_mb = current_mb;
                    println!("[Transcoding] Progress: {:.1} MB sent...", total_bytes as f64 / (1024.0 * 1024.0));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                eprintln!("[Transcoding] FFmpeg read error: {}", e);
                session.kill("read error");
                break;
            }
        }
    }
    
    let _ = stream.flush();
    // Note: We don't kill here if we finished naturally - process already exited
}

/// Write data to stream with TIMEOUT for stall detection
/// Returns false on fatal disconnect OR if stalled for 30 seconds (client likely seeked)
/// This prevents zombie FFmpeg processes when client abandons connection
fn write_with_backpressure(stream: &mut TcpStream, data: &[u8]) -> bool {
    use std::io::ErrorKind;
    
    // Stall detection: if no progress for 30 seconds, assume client is gone
    const MAX_STALL_ITERATIONS: u32 = 600; // 600 * 50ms = 30 seconds
    let mut stall_count: u32 = 0;
    let mut written = 0;
    
    while written < data.len() {
        match stream.write(&data[written..]) {
            Ok(0) => {
                // Zero bytes written - buffer full, wait and retry
                stall_count += 1;
                if stall_count >= MAX_STALL_ITERATIONS {
                    eprintln!("[Transcoding] Write stalled for 30s, aborting (client likely seeked)");
                    return false;
                }
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Ok(n) => {
                // Progress - advance buffer and reset stall counter
                written += n;
                stall_count = 0;
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                // Backpressure - client buffer full, wait and retry
                stall_count += 1;
                if stall_count >= MAX_STALL_ITERATIONS {
                    eprintln!("[Transcoding] Write stalled for 30s (WouldBlock), aborting");
                    return false;
                }
                thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(ref e) if e.kind() == ErrorKind::Interrupted => {
                // Interrupted by signal, retry immediately
                continue;
            }
            // FATAL ERRORS - client actually disconnected
            Err(ref e) if e.kind() == ErrorKind::BrokenPipe => {
                return false;
            }
            Err(ref e) if e.kind() == ErrorKind::ConnectionReset => {
                return false;
            }
            Err(ref e) if e.kind() == ErrorKind::ConnectionAborted => {
                return false;
            }
            Err(e) => {
                // Unknown error - log but keep trying (could be transient)
                eprintln!("[Transcoding] Write warning: {}", e);
                stall_count += 1;
                if stall_count >= MAX_STALL_ITERATIONS {
                    return false;
                }
                thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
        }
    }
    true
}

/// Parse mode parameter from URL (mode=direct or mode=transcode)
fn parse_mode_param(request_line: &str) -> Option<&str> {
    if request_line.contains("mode=direct") {
        Some("direct")
    } else if request_line.contains("mode=transcode") {
        Some("transcode")
    } else {
        None
    }
}

/// SMART ROUTER: Routes based on mode param - NO PROBE per request
fn handle_video_request(
    stream: TcpStream, 
    video_path: &PathBuf,
    session: crate::infrastructure::media::ffmpeg_session::FfmpegSession
) {
    // Read request
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    
    // Read headers
    let mut range_header: Option<String> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
            break;
        }
        if line.to_lowercase().starts_with("range:") {
            range_header = Some(line.trim().to_string());
        }
    }
    
    // Parse mode from URL (set at construction time - NO PROBE HERE)
    let mode = parse_mode_param(&request_line);
    let seek_seconds = parse_seek_param(&request_line);
    
    match mode {
        Some("direct") => {
            // FAST PATH: No probe, just serve file with Range support
            handle_direct_play(stream, video_path, range_header);
        }
        Some("transcode") => {
            // Transcode path - use centralized session
            handle_transcoding(stream, video_path, seek_seconds, None, session);
        }
        None => {
            // Fallback (shouldn't happen with new URL pattern)
            // Only probe here as safety fallback
            println!("[SmartServer] FALLBACK: No mode param, probing...");
            let info = probe_video_codecs_internal(video_path);
            if is_codec_webview_compatible(&info) {
                handle_direct_play(stream, video_path, range_header);
            } else {
                handle_transcoding(stream, video_path, seek_seconds, info.duration, session);
            }
        }
        _ => {
            handle_direct_play(stream, video_path, range_header);
        }
    }
}

fn rect_to_physical(rect: egui::Rect, scale: f32) -> egui::Rect {
    egui::Rect {
        min: egui::pos2(rect.min.x * scale, rect.min.y * scale),
        max: egui::pos2(rect.max.x * scale, rect.max.y * scale),
    }
}

/// Format time in seconds to MM:SS format
pub fn format_time(seconds: f64) -> String {
    if seconds.is_nan() || seconds < 0.0 {
        return "0:00".to_string();
    }
    let total_secs = seconds as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    format!("{}:{:02}", mins, secs)
}
