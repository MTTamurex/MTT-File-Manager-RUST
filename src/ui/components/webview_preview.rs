use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::io::{Read, Write, BufReader, BufRead};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use eframe::egui;
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOW, FindWindowExW};
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
    
    // Player state
    pub show_player: bool,     // false = show thumbnail, true = show video
    pub play_on_init: bool,    // if true, play as soon as webview is ready
    pub state: Arc<Mutex<VideoState>>,
    pub is_visible: bool,      // Track intended visibility state
    
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
            show_player: false,
            play_on_init: false,
            state: Arc::new(Mutex::new(VideoState {
                is_playing: false,
                current_time: 0.0,
                duration: 0.0,
                volume: 1.0,
                is_muted: false,
            })),
            is_visible: true, 
            #[cfg(target_os = "windows")]
            webview_hwnd: Arc::new(Mutex::new(None)),
        }
    }

    /// Get current playback state
    pub fn get_state(&self) -> VideoState {
        self.state.lock().unwrap().clone()
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
            let is_playing = self.state.lock().unwrap().is_playing;
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
        if let Ok(mut state) = self.state.lock() {
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
        if let Ok(mut state) = self.state.lock() {
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

    fn start_video_server(&mut self) -> Option<u16> {
        let video_path = dunce::canonicalize(&self.path).unwrap_or(self.path.clone());
        
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
        self._server_shutdown = Some(shutdown_tx);
        
        // Spawn server thread
        thread::spawn(move || {
            listener.set_nonblocking(true).ok();
            
            loop {
                // Check for shutdown
                if shutdown_rx.try_recv().is_ok() {
                    println!("[VideoServer] Shutting down");
                    break;
                }
                
                // Accept connections
                match listener.accept() {
                    Ok((stream, _)) => {
                        let path = video_path.clone();
                        thread::spawn(move || {
                            handle_video_request(stream, &path);
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
        
        Some(port)
    }

    fn init_webview(&mut self, _ctx: &egui::Context, _ui: &egui::Ui, window: &dyn raw_window_handle::HasWindowHandle) {
        // Start local video server
        let port = match self.start_video_server() {
            Some(p) => p,
            None => {
                eprintln!("[WebviewPreview] Failed to start video server");
                return;
            }
        };
        self.server_port = Some(port);
        
        let video_url = format!("http://127.0.0.1:{}/video.mp4", port);
        println!("[WebviewPreview] Video URL: {}", video_url);
        
        // Check if transcoding is required for this file
        let needs_transcoding = is_transcode_required(&self.path);
        
        // Get duration for transcoded files (needed for seek bar)
        let duration = if needs_transcoding {
            get_video_duration_ffprobe(&self.path).unwrap_or(0.0)
        } else {
            0.0 // Will be detected from file metadata
        };
        
        // Clone state for IPC handler
        let state_clone = self.state.clone();
        
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
            const url = seekSeconds > 0 ? baseUrl + '?seek=' + seekSeconds.toFixed(2) : baseUrl;
            console.log("Loading:", url);
            video.src = url;
            video.load();
            video.play().catch(e => console.log("Autoplay blocked:", e));
            isReloading = false;
        }}
        
        // Initial load
        loadVideoAt(0);
        
        // Report state to Rust every 250ms (with offset correction)
        setInterval(() => {{
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
        }}, 250);
        
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

        // Report state to Rust every 250ms
        setInterval(() => {{
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
        }}, 250);
        
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
                                    if let Ok(mut state) = state_clone.lock() {
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
                                    if let Ok(mut state) = state_clone.lock() {
                                        state.is_playing = true;
                                    }
                                }
                                "pause" => {
                                    if let Ok(mut state) = state_clone.lock() {
                                        state.is_playing = false;
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
                    if let Ok(mut state) = self.state.lock() {
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
        
        // Reserve space for the video
        let available = ui.available_size();
        let preview_height = (available.x * 0.6).min(300.0);
        let size = egui::vec2(available.x, preview_height);
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

        // Lazy init
        if self.webview.is_none() {
            if let Some(frame) = frame {
                use raw_window_handle::HasWindowHandle;
                if let Ok(handle) = frame.window_handle() {
                    self.init_webview(ui.ctx(), ui, &handle);
                }
            } else {
                ui.ctx().request_repaint();
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
        
        // Request repaint to keep state updated
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
    
    pub fn try_init(&mut self, window: &dyn raw_window_handle::HasWindowHandle, ctx: &egui::Context, ui: &egui::Ui) {
        if self.webview.is_none() {
            self.init_webview(ctx, ui, window);
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

/// Get video duration in seconds using ffprobe
fn get_video_duration_ffprobe(video_path: &PathBuf) -> Option<f64> {
    let file_path = video_path.to_string_lossy().to_string();
    
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
    
    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim().parse::<f64>().ok()
        }
        Err(e) => {
            eprintln!("[FFprobe] Failed to get duration: {}", e);
            None
        }
    }
}

/// Handle MKV (and other) files by transcoding to MP4 on-the-fly using FFmpeg
/// Returns true if transcoding was handled, false if FFmpeg is not available
/// seek_seconds: Optional seek position in seconds (from ?seek=X query param)
/// duration: Optional pre-fetched video duration (to avoid calling ffprobe again)
fn handle_mkv_transcoding(
    mut stream: TcpStream, 
    video_path: &PathBuf, 
    seek_seconds: Option<f64>,
    duration: Option<f64>
) -> bool {
    let file_path = video_path.to_string_lossy().to_string();
    
    let seek_str = seek_seconds.map(|s| format!("{:.2}", s)).unwrap_or_default();
    if seek_seconds.is_some() {
        println!("[Transcoding] Starting FFmpeg for: {} (seek: {}s)", file_path, seek_str);
    } else {
        println!("[Transcoding] Starting FFmpeg for: {}", file_path);
    }
    
    // Build FFmpeg command
    let mut cmd = Command::new("ffmpeg");
    
    // Add seek BEFORE input for fast seek (input seeking)
    if let Some(seek) = seek_seconds {
        cmd.args(&["-ss", &format!("{:.2}", seek)]);
    }
    
    cmd.args(&[
        "-i", &file_path,          // Input file
        "-c:v", "libx264",         // Transcode to H.264
        "-preset", "ultrafast",    // Prioritize speed over compression
        "-tune", "zerolatency",    // Optimize for streaming
        "-c:a", "aac",             // Transcode audio to AAC
        "-f", "mp4",               // Output format: MP4
        "-movflags", "frag_keyframe+empty_moov+faststart", // Enable streaming
        "pipe:1"                   // Output to stdout
    ]);
    
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped()); // Capture stderr for debugging
    
    // Windows-specific: Hide the console window
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    
    // Try to spawn FFmpeg
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[Transcoding] Failed to spawn FFmpeg: {}. Is ffmpeg in PATH?", e);
            let response = "HTTP/1.1 500 Internal Server Error\r\n\
                Content-Type: text/plain\r\n\
                Content-Length: 25\r\n\
                \r\n\
                FFmpeg not found in PATH";
            let _ = stream.write_all(response.as_bytes());
            return false;
        }
    };
    
    // Spawn a thread to read and log stderr
    let stderr = child.stderr.take();
    thread::spawn(move || {
        if let Some(mut err) = stderr {
            let mut err_output = String::new();
            if err.read_to_string(&mut err_output).is_ok() && !err_output.is_empty() {
                // Only print first 500 chars to avoid spam
                let truncated: String = err_output.chars().take(500).collect();
                eprintln!("[FFmpeg stderr] {}", truncated);
            }
        }
    });
    
    // Take stdout from the child process
    let mut ffmpeg_stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            eprintln!("[Transcoding] Failed to capture FFmpeg stdout");
            let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            return false;
        }
    };
    
    // Send HTTP headers with optional duration header
    let mut headers = String::from("HTTP/1.1 200 OK\r\n\
        Content-Type: video/mp4\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Cache-Control: no-cache\r\n\
        Connection: close\r\n");
    
    // Add duration header if available
    if let Some(dur) = duration {
        headers.push_str(&format!("X-Video-Duration: {:.2}\r\n", dur));
    }
    headers.push_str("\r\n");
    
    if stream.write_all(headers.as_bytes()).is_err() {
        eprintln!("[Transcoding] Failed to write headers");
        let _ = child.kill();
        return false;
    }
    
    // Set socket to non-blocking to detect backpressure
    let _ = stream.set_nonblocking(false);
    
    // Stream FFmpeg output with backpressure handling
    let mut buffer = [0u8; 64 * 1024]; // 64KB buffer
    let mut total_bytes: u64 = 0;
    let mut last_logged_mb: u64 = 0;
    
    loop {
        match ffmpeg_stdout.read(&mut buffer) {
            Ok(0) => {
                // EOF - FFmpeg finished producing data
                let _ = stream.flush();
                println!("[Transcoding] Stream complete. Total bytes sent: {} ({:.1} MB)", 
                    total_bytes, total_bytes as f64 / (1024.0 * 1024.0));
                break;
            }
            Ok(n) => {
                // Write with retry for backpressure
                let mut written = 0;
                let data = &buffer[..n];
                
                while written < data.len() {
                    match stream.write(&data[written..]) {
                        Ok(0) => {
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Ok(w) => {
                            written += w;
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                            continue;
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe 
                            || e.kind() == std::io::ErrorKind::ConnectionReset 
                            || e.kind() == std::io::ErrorKind::ConnectionAborted => {
                            eprintln!("[Transcoding] Client disconnected: {} (sent {} bytes)", e, total_bytes);
                            let _ = child.kill();
                            return true;
                        }
                        Err(e) => {
                            eprintln!("[Transcoding] Write warning: {} (retrying...)", e);
                            thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                }
                
                total_bytes += n as u64;
                
                // Log progress every 5MB
                let current_mb = total_bytes / (5 * 1024 * 1024);
                if current_mb > last_logged_mb {
                    last_logged_mb = current_mb;
                    println!("[Transcoding] Progress: {:.1} MB sent...", total_bytes as f64 / (1024.0 * 1024.0));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                continue;
            }
            Err(e) => {
                eprintln!("[Transcoding] FFmpeg read error: {}", e);
                break;
            }
        }
    }
    
    let _ = stream.flush();
    
    // Wait for FFmpeg to finish
    match child.wait() {
        Ok(status) => println!("[Transcoding] FFmpeg exited with: {}", status),
        Err(e) => eprintln!("[Transcoding] Failed to wait for FFmpeg: {}", e),
    }
    
    true
}

/// Check if the file extension requires transcoding (not natively supported by WebView2)
fn is_transcode_required(path: &PathBuf) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    path_str.ends_with(".mkv") 
        || path_str.ends_with(".avi") 
        || path_str.ends_with(".wmv")
        || path_str.ends_with(".ogm")
        || path_str.ends_with(".flv")
        || path_str.ends_with(".rmvb")
        || path_str.ends_with(".rm")
}

/// Parse seek parameter from URL query string (e.g., /video.mp4?seek=120.5)
fn parse_seek_param(request_line: &str) -> Option<f64> {
    println!("[DEBUG] Parsing request line: {}", request_line.trim());
    
    // Find ?seek= or &seek= in the URL
    if let Some(query_start) = request_line.find('?') {
        let query = &request_line[query_start..];
        println!("[DEBUG] Query string found: {}", query);
        
        // Look for seek= parameter
        for param in query.split('&') {
            let param = param.trim_start_matches('?');
            if let Some(value) = param.strip_prefix("seek=") {
                // Remove any trailing path/protocol parts
                let value = value.split_whitespace().next().unwrap_or(value);
                println!("[DEBUG] Found seek value: '{}'", value);
                if let Ok(parsed) = value.parse::<f64>() {
                    println!("[DEBUG] Successfully parsed seek to: {} seconds", parsed);
                    return Some(parsed);
                } else {
                    println!("[DEBUG] Failed to parse seek value as f64");
                }
            }
        }
    } else {
        println!("[DEBUG] No query string in request");
    }
    None
}

fn handle_video_request(mut stream: TcpStream, video_path: &PathBuf) {
    use std::io::Seek;
    use std::fs::File;
    
    // Read request line first to get any query parameters
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    
    println!("[VideoServer] Received request: {}", request_line.trim());
    
    // Parse seek parameter if present
    let seek_seconds = parse_seek_param(&request_line);
    
    if let Some(seek) = seek_seconds {
        println!("[VideoServer] SEEK requested: {} seconds", seek);
    } else {
        println!("[VideoServer] No seek parameter in request");
    }
    
    // Check if transcoding is required for this file type
    if is_transcode_required(video_path) {
        // Read and discard remaining HTTP headers
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                break;
            }
        }
        
        // Get video duration via ffprobe (only on first request without seek)
        let duration = if seek_seconds.is_none() {
            get_video_duration_ffprobe(video_path)
        } else {
            None // Don't re-fetch on seek requests
        };
        
        // Handle via FFmpeg transcoding with seek support
        handle_mkv_transcoding(stream, video_path, seek_seconds, duration);
        return;
    }
    
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    
    // Read headers to find Range
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
    
    // Get file size without reading entire file
    let mut file = match File::open(video_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[VideoServer] Failed to open file: {}", e);
            let response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            return;
        }
    };
    
    let total_size = match file.metadata() {
        Ok(m) => m.len() as usize,
        Err(_) => {
            let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
            return;
        }
    };
    
    // Determine MIME type
    let path_str = video_path.to_string_lossy();
    let mime = if path_str.ends_with(".mp4") { "video/mp4" }
        else if path_str.ends_with(".webm") { "video/webm" }
        else if path_str.ends_with(".mkv") { "video/x-matroska" }
        else { "video/mp4" };
    
    // Handle Range requests for video seeking
    if let Some(range) = range_header {
        if let Some(range_spec) = range.strip_prefix("Range: bytes=") {
            let parts: Vec<&str> = range_spec.trim().split('-').collect();
            let start: usize = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
            // Limit chunk size to 2MB for faster response
            let max_chunk = 2 * 1024 * 1024;
            let requested_end: usize = parts.get(1)
                .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
                .unwrap_or((start + max_chunk).min(total_size - 1));
            let end = requested_end.min(total_size - 1);
            
            let chunk_size = end - start + 1;
            
            // Seek to start position and read only the chunk
            if file.seek(std::io::SeekFrom::Start(start as u64)).is_err() {
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes());
                return;
            }
            
            let mut buffer = vec![0u8; chunk_size];
            match file.read_exact(&mut buffer) {
                Ok(_) => {},
                Err(_) => {
                    // Try partial read
                    let _ = file.seek(std::io::SeekFrom::Start(start as u64));
                    buffer.resize(chunk_size, 0);
                    if let Ok(n) = file.read(&mut buffer) {
                        buffer.truncate(n);
                    }
                }
            }
            
            let content_range = format!("bytes {}-{}/{}", start, start + buffer.len() - 1, total_size);
            
            let response = format!(
                "HTTP/1.1 206 Partial Content\r\n\
                Content-Type: {}\r\n\
                Content-Length: {}\r\n\
                Content-Range: {}\r\n\
                Accept-Ranges: bytes\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: keep-alive\r\n\
                \r\n",
                mime, buffer.len(), content_range
            );
            
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&buffer);
        }
    } else {
        // For non-range requests, send headers indicating we support ranges
        // but only send first chunk to allow quick start
        let initial_chunk = (1024 * 1024).min(total_size); // 1MB initial
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
