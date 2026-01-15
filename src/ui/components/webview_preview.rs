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
            let _ = webview.evaluate_script(&format!(
                "document.getElementById('player').currentTime = {}", time
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
        
        // Clone state for IPC handler
        let state_clone = self.state.clone();
        
        // HTML without native controls - video will autoplay when loaded
        let html_content = format!(r#"<!DOCTYPE html>
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
</html>"#, video_url);

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

/// Check if the file extension requires transcoding (not natively supported by WebView2)
fn is_transcode_required(path: &PathBuf) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    // Formats not natively supported by WebView2/Edge
    path_str.ends_with(".mkv") 
        || path_str.ends_with(".avi") 
        || path_str.ends_with(".wmv")
        || path_str.ends_with(".ogm")
        || path_str.ends_with(".flv")
        || path_str.ends_with(".rmvb")
        || path_str.ends_with(".rm")
}

/// Handle MKV (and other) files by transcoding to MP4 on-the-fly using FFmpeg
/// Returns true if transcoding was handled, false if FFmpeg is not available
fn handle_mkv_transcoding(mut stream: TcpStream, video_path: &PathBuf) -> bool {
    let file_path = video_path.to_string_lossy().to_string();
    
    println!("[Transcoding] Starting FFmpeg for: {}", file_path);
    
    // Build FFmpeg command
    let mut cmd = Command::new("ffmpeg");
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
    
    // Send HTTP headers - using raw streaming (no chunked, no content-length)
    // WebView2 should handle this with Connection: close
    let response_headers = "HTTP/1.1 200 OK\r\n\
        Content-Type: video/mp4\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Cache-Control: no-cache\r\n\
        Connection: close\r\n\
        \r\n";
    
    if stream.write_all(response_headers.as_bytes()).is_err() {
        eprintln!("[Transcoding] Failed to write headers");
        let _ = child.kill();
        return false;
    }
    
    // Set socket to non-blocking to detect backpressure
    // We'll handle blocking manually with retries
    let _ = stream.set_nonblocking(false); // Keep blocking for simplicity but handle errors
    
    // Stream FFmpeg output with backpressure handling
    let mut buffer = [0u8; 64 * 1024]; // 64KB buffer
    let mut total_bytes: u64 = 0;
    let mut last_logged_mb: u64 = 0;
    
    loop {
        match ffmpeg_stdout.read(&mut buffer) {
            Ok(0) => {
                // EOF - FFmpeg finished producing data
                // Flush any remaining data
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
                            // Write returned 0 - likely backpressure, wait and retry
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Ok(w) => {
                            written += w;
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // Backpressure - client buffer full, wait and retry
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                            // Interrupted, just retry immediately
                            continue;
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe 
                            || e.kind() == std::io::ErrorKind::ConnectionReset 
                            || e.kind() == std::io::ErrorKind::ConnectionAborted => {
                            // Fatal: Client actually disconnected
                            eprintln!("[Transcoding] Client disconnected: {} (sent {} bytes)", e, total_bytes);
                            let _ = child.kill();
                            return true;
                        }
                        Err(e) => {
                            // Other error - log and try to continue
                            eprintln!("[Transcoding] Write warning: {} (retrying...)", e);
                            thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                }
                
                total_bytes += n as u64;
                
                // Log progress every 5MB (avoid spam)
                let current_mb = total_bytes / (5 * 1024 * 1024);
                if current_mb > last_logged_mb {
                    last_logged_mb = current_mb;
                    println!("[Transcoding] Progress: {:.1} MB sent...", total_bytes as f64 / (1024.0 * 1024.0));
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                // Read interrupted, just retry
                continue;
            }
            Err(e) => {
                eprintln!("[Transcoding] FFmpeg read error: {}", e);
                break;
            }
        }
    }
    
    // Flush final data
    let _ = stream.flush();
    
    // Wait for FFmpeg to finish
    match child.wait() {
        Ok(status) => println!("[Transcoding] FFmpeg exited with: {}", status),
        Err(e) => eprintln!("[Transcoding] Failed to wait for FFmpeg: {}", e),
    }
    
    true
}

fn handle_video_request(mut stream: TcpStream, video_path: &PathBuf) {
    use std::io::Seek;
    use std::fs::File;
    
    // Check if transcoding is required for this file type
    if is_transcode_required(video_path) {
        // Read and discard the HTTP request headers first
        let reader = BufReader::new(&stream);
        for line in reader.lines().map_while(Result::ok) {
            if line.is_empty() {
                break;
            }
        }
        
        // Handle via FFmpeg transcoding
        handle_mkv_transcoding(stream, video_path);
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
