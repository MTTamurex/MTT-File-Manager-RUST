//! Standalone dedicated media player mode (separate process).
//!
//! When the user clicks "detach" on the docked player, the main app
//! spawns a new process (`--video-player <path> [--position <secs>] [--volume <vol>]`)
//! that runs an independent mpv window (borderless, with OSC window controls).
//!
//! mpv creates its own native window (no `wid` embedding), so all native features
//! work: keyboard shortcuts, OSC, window management via OSC buttons.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use rfd::FileDialog;

/// Base OSC script-opts for the standalone player.
const STANDALONE_OSC_BASE_SCRIPT_OPTS: &str =
    "osc-scalewindowed=1,osc-scalefullscreen=1,osc-scaleforcedwindow=1,osc-windowcontrols=yes";

/// Maximum file size for the video player (50 GB).
const MAX_VIDEO_FILE_SIZE: u64 = 50 * 1024 * 1024 * 1024;

fn apply_saved_locale() {
    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MTT-File-Manager")
        .join("thumbnails");

    if let Ok(cache) = crate::infrastructure::disk_cache::ThumbnailDiskCache::new(cache_dir) {
        if let Some(language) = cache.get_preference("language") {
            rust_i18n::set_locale(&language);
        }
    }
}

pub(crate) fn current_mpv_osc_language() -> &'static str {
    match &*rust_i18n::locale() {
        "pt-BR" | "pt" | "ptbr" => "pt-BR",
        "en" | "eng" | "en-US" => "en",
        _ => "en",
    }
}

pub(crate) fn build_mpv_osc_script_opts(base_opts: &str) -> String {
    format!("{base_opts},osc-language={}", current_mpv_osc_language())
}

/// SEC: Validate the video/audio path before opening. Blocks null bytes, path traversal,
/// UNC/network paths, non-media extensions, and oversized files.
fn validate_video_path(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    // 1. Null bytes
    if path_str.contains('\0') {
        return Err("Path contains null bytes".into());
    }

    // 2. Path traversal
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(format!(
                "Path traversal component '{}' not allowed",
                component.as_os_str().to_string_lossy()
            ));
        }
    }

    // 3. Block UNC / network paths
    if path_str.starts_with("\\\\") || path_str.starts_with("//") || path_str.starts_with("\\\\?\\UNC\\") {
        return Err("Network/UNC paths are not allowed".into());
    }

    // 4. Extension whitelist (video + audio, since mpv plays both)
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ext_lower = ext.to_lowercase();
    if !crate::infrastructure::windows::is_video_extension(&ext_lower)
        && !crate::infrastructure::windows::is_audio_extension(&ext_lower)
    {
        return Err(format!("Unsupported media extension: '{}'", ext));
    }

    // 5. File existence
    if !path.is_file() {
        return Err(format!("File not found: '{}'", path.display()));
    }

    // 6. File size
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_VIDEO_FILE_SIZE {
            return Err(format!(
                "File too large: {:.1} GB (max {} GB)",
                meta.len() as f64 / (1024.0 * 1024.0 * 1024.0),
                MAX_VIDEO_FILE_SIZE / (1024 * 1024 * 1024)
            ));
        }
    }

    Ok(())
}

/// Spawn a standalone video player process for the given file.
///
/// Returns the `Child` handle so the caller can track/kill the process.
pub fn open_video_player(path: PathBuf, position: f64, volume: f32) -> Option<Child> {
    // SEC: Validate path before spawning child process.
    if let Err(e) = validate_video_path(&path) {
        log::error!("[VIDEO-PLAYER] path validation failed for '{}': {}", path.display(), e);
        return None;
    }

    let exe = match std::env::current_exe() {
        Ok(v) => v,
        Err(err) => {
            log::error!(
                "[VIDEO-PLAYER] failed to locate current executable for spawn: {}",
                err
            );
            return None;
        }
    };

    let spawn_result = Command::new(exe)
        .arg("--video-player")
        .arg(&path)
        .arg("--position")
        .arg(position.to_string())
        .arg("--volume")
        .arg(volume.to_string())
        .spawn();

    match spawn_result {
        Ok(child) => Some(child),
        Err(err) => {
            log::error!(
                "[VIDEO-PLAYER] failed to spawn standalone player for '{}': {}",
                path.display(),
                err
            );
            None
        }
    }
}

/// Resolve the `mpv_ui/portable_config` directory (same logic as MpvPreview).
fn resolve_mpv_ui_config_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // SEC: Do NOT search CWD for config — an attacker could plant a malicious
    // config directory if the app is launched from an untrusted location.
    // Only search relative to the executable's own directory.
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
        .find(|dir| {
            let scripts = dir.join("scripts");
            scripts.join("modernH.lua").is_file() && scripts.join("vsr.lua").is_file()
        })
}

/// Convert a Windows path to forward-slash form for mpv options.
fn mpv_path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn pick_subtitle_for_video(video_path: &std::path::Path) -> Option<PathBuf> {
    let mut dialog = FileDialog::new().add_filter(
        rust_i18n::t!("video.subtitle_filter").to_string(),
        &["srt", "ass", "ssa", "vtt", "sub", "sup", "idx", "mks"],
    );

    if let Some(parent) = video_path.parent() {
        dialog = dialog.set_directory(parent);
    }

    dialog.pick_file()
}

fn load_external_subtitle_for_standalone(
    mpv: &mut mpv::Mpv,
    video_path: &std::path::Path,
) -> Result<bool, String> {
    let Some(subtitle_path) = pick_subtitle_for_video(video_path) else {
        return Ok(false);
    };

    let subtitle_str = subtitle_path.to_string_lossy().to_string();
    mpv.command("sub-add", &[&subtitle_str, "select"])
        .map_err(|e| format!("{}", rust_i18n::t!("video.subtitle_load_failed", error = format!("{:?}", e))))?;

    let file_name = subtitle_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or(subtitle_str);
    let loaded_msg = rust_i18n::t!("video.subtitle_loaded", name = file_name).to_string();
    let _ = mpv.command("show-text", &[&loaded_msg, "2000"]);

    Ok(true)
}

/// Load app icons from the current executable.
#[cfg(target_os = "windows")]
fn load_app_icons() -> Option<(isize, isize)> {
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, IMAGE_ICON, LR_SHARED,
    };
    use windows::core::PCWSTR;

    // Load small icon (16x16) and big icon (32x32) from exe resource
    let hmodule = unsafe {
        windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default()
    };

    let hicon_small = unsafe {
        LoadImageW(
            Some(hmodule.into()),
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            16,
            16,
            LR_SHARED,
        )
        .ok()
    };

    let hicon_big = unsafe {
        LoadImageW(
            Some(hmodule.into()),
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            32,
            32,
            LR_SHARED,
        )
        .ok()
    };

    let small_raw = hicon_small.map(|h| h.0 as isize).unwrap_or(0);
    let big_raw = hicon_big.map(|h| h.0 as isize).unwrap_or(0);

    if small_raw != 0 || big_raw != 0 {
        return Some((small_raw, big_raw));
    }

    let exe_path = std::env::current_exe().ok()?;
    let wide: Vec<u16> = exe_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut h_large = [windows::Win32::UI::WindowsAndMessaging::HICON::default()];
    let mut h_small = [windows::Win32::UI::WindowsAndMessaging::HICON::default()];

    let count = unsafe {
        windows::Win32::UI::Shell::ExtractIconExW(
            PCWSTR(wide.as_ptr()),
            0,
            Some(h_large.as_mut_ptr()),
            Some(h_small.as_mut_ptr()),
            1,
        )
    };

    if count > 0 {
        let fallback_small = if !h_small[0].is_invalid() {
            h_small[0].0 as isize
        } else {
            0isize
        };
        let fallback_big = if !h_large[0].is_invalid() {
            h_large[0].0 as isize
        } else {
            0isize
        };
        if fallback_small != 0 || fallback_big != 0 {
            return Some((fallback_small, fallback_big));
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn try_get_mpv_hwnd(mpv: &mpv::Mpv) -> Option<windows::Win32::Foundation::HWND> {
    if let Ok(raw_hwnd) = mpv.get_property::<i64>("window-id") {
        if raw_hwnd > 0 {
            return Some(windows::Win32::Foundation::HWND(raw_hwnd as *mut std::ffi::c_void));
        }
    }

    if let Ok(raw_hwnd) = mpv.get_property::<String>("window-id") {
        let trimmed = raw_hwnd.trim();
        if !trimmed.is_empty() {
            if let Ok(parsed) = trimmed.parse::<isize>() {
                if parsed > 0 {
                    return Some(windows::Win32::Foundation::HWND(parsed as *mut std::ffi::c_void));
                }
            }
            if let Some(hex) = trimmed.strip_prefix("0x") {
                if let Ok(parsed) = isize::from_str_radix(hex, 16) {
                    if parsed > 0 {
                        return Some(windows::Win32::Foundation::HWND(parsed as *mut std::ffi::c_void));
                    }
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn apply_icon_to_hwnd(hwnd: windows::Win32::Foundation::HWND, small_raw: isize, big_raw: isize) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageW, SetClassLongPtrW, ICON_BIG, ICON_SMALL, GCLP_HICON, GCLP_HICONSM,
        WM_SETICON,
    };

    unsafe {
        if small_raw != 0 {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(small_raw)),
            );
            let _ = SetClassLongPtrW(hwnd, GCLP_HICONSM, small_raw);
        }

        if big_raw != 0 {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(big_raw)),
            );
            let _ = SetClassLongPtrW(hwnd, GCLP_HICON, big_raw);
        }
    }
}

/// Set the mpv window icon to our app icon.
#[cfg(target_os = "windows")]
fn set_mpv_window_icon(mpv: &mpv::Mpv) {
    use std::thread;
    use std::time::Duration;
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

    let Some((small_raw, big_raw)) = load_app_icons() else {
        log::warn!("[VIDEO-PLAYER] Failed to load app icon from resources and exe");
        return;
    };

    for attempt in 1..=10 {
        if let Some(hwnd) = try_get_mpv_hwnd(mpv) {
            apply_icon_to_hwnd(hwnd, small_raw, big_raw);
            log::info!(
                "[VIDEO-PLAYER] Applied app icon to mpv hwnd=0x{:x} attempt={}",
                hwnd.0 as usize,
                attempt
            );
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let current_pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
    let data = (current_pid, small_raw, big_raw);

    unsafe {
        let _ = EnumWindows(
            Some(enum_set_icon),
            windows::Win32::Foundation::LPARAM(
                &data as *const (u32, isize, isize) as isize,
            ),
        );
    }

    log::debug!("[VIDEO-PLAYER] Applied app icon via pid enumeration fallback");
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_set_icon(
    hwnd: windows::Win32::Foundation::HWND,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::core::BOOL {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};

    let data = &*(lparam.0 as *const (u32, isize, isize));
    let target_pid = data.0;
    let hicon_small = data.1;
    let hicon_big = data.2;

    let mut window_pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut window_pid));

    if window_pid == target_pid && IsWindowVisible(hwnd).as_bool() {
        apply_icon_to_hwnd(hwnd, hicon_small, hicon_big);
    }

    true.into()
}

/// Entry point for the standalone video player process.
///
/// Creates a native mpv window (borderless) with the custom OSC providing
/// window controls (close, minimize, maximize). No eframe wrapper needed.
pub fn run_standalone(path: PathBuf, position: f64, volume: f32) -> eframe::Result<()> {
    apply_saved_locale();

    // SEC: Validate again in child process (defense in depth).
    if let Err(e) = validate_video_path(&path) {
        log::error!("[VIDEO-PLAYER] path validation failed in standalone: {}", e);
        return Ok(());
    }

    let title_name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "Media Player".to_string());

    let config_dir = resolve_mpv_ui_config_dir();
    if let Some(dir) = &config_dir {
        log::info!(
            "[VIDEO-PLAYER] Using mpv config dir: {}",
            dir.to_string_lossy()
        );
    } else {
        log::warn!(
            "[VIDEO-PLAYER] mpv_ui/portable_config not found (with scripts/modernH.lua + scripts/vsr.lua); OSC/VSR may not load"
        );
    }

    let osc_script_opts = build_mpv_osc_script_opts(STANDALONE_OSC_BASE_SCRIPT_OPTS);

    // Create mpv with initializer options (set before mpv_initialize).
    let mpv = mpv::Mpv::with_initializer(|init| {
        // Load config FIRST so mpv.conf (vo, gpu-api, hwdec, etc.) is applied.
        // Options set after this will override mpv.conf values.
        if let Some(dir) = &config_dir {
            let dir_str = mpv_path_string(dir.as_path());
            let _ = init.set_option("config", true);
            let _ = init.set_option("config-dir", dir_str.as_str());
        }

        // Borderless window — OSC provides the window controls
        if let Err(e) = init.set_option("border", false) {
            log::warn!("[VIDEO-PLAYER] Failed to set border=no: {:?}", e);
        }

        // Load custom OSC script
        if let Err(e) = init.set_option("load-scripts", true) {
            log::warn!("[VIDEO-PLAYER] Failed to set load-scripts: {:?}", e);
        }
        if let Err(e) = init.set_option("osc", false) {
            log::warn!("[VIDEO-PLAYER] Failed to set osc=no: {:?}", e);
        }
        if let Err(e) = init.set_option("input-default-bindings", true) {
            log::warn!("[VIDEO-PLAYER] Failed to set input-default-bindings: {:?}", e);
        }
        if let Err(e) = init.set_option("input-vo-keyboard", true) {
            log::warn!("[VIDEO-PLAYER] Failed to set input-vo-keyboard: {:?}", e);
        }
        if let Err(e) = init.set_option("input-cursor", true) {
            log::warn!("[VIDEO-PLAYER] Failed to set input-cursor: {:?}", e);
        }
        if let Err(e) = init.set_option("cursor-autohide", 1000_i64) {
            log::warn!("[VIDEO-PLAYER] Failed to set cursor-autohide: {:?}", e);
        }
        if let Err(e) = init.set_option("script-opts", osc_script_opts.as_str()) {
            log::warn!("[VIDEO-PLAYER] Failed to set script-opts: {:?}", e);
        }

        Ok(())
    });

    let mut mpv = match mpv {
        Ok(m) => m,
        Err(e) => {
            log::error!("[VIDEO-PLAYER] Failed to create mpv instance: {:?}", e);
            return Ok(());
        }
    };

    // --- Runtime properties (after mpv_initialize, before loadfile) ---

    // D3D11 pipeline for NVIDIA RTX VSR — must be set via set_property after init,
    // same sequencing as the embedded player (MpvPreview). Setting these in
    // mpv.conf or set_option causes the VO to initialize during mpv_initialize()
    // before the hwdec interop is ready, leaving hwdec-current empty.
    let _ = mpv.set_property("vo", "gpu-next");
    let _ = mpv.set_property("gpu-api", "d3d11");
    let _ = mpv.set_property("gpu-context", "d3d11");
    let _ = mpv.set_property("hwdec", "d3d11va");

    // Playback stability
    let _ = mpv.set_property("force-window", true);
    let _ = mpv.set_property("video-sync", "audio");
    let _ = mpv.set_property("interpolation", false);
    let _ = mpv.set_property("tscale", "linear");
    let _ = mpv.set_property("framedrop", "vo");
    let _ = mpv.set_property("keep-open", "always");

    // Cache/demuxer settings
    let _ = mpv.set_property("cache", "yes");
    let _ = mpv.set_property("cache-secs", 12.0_f64);
    let _ = mpv.set_property("demuxer-readahead-secs", 6.0_f64);
    let _ = mpv.set_property("demuxer-max-bytes", 48_i64 * 1024 * 1024);
    let _ = mpv.set_property("demuxer-max-back-bytes", 12_i64 * 1024 * 1024);

    // Volume (mpv uses 0-100 scale)
    let _ = mpv.set_property("volume", ((volume * 100.0) as i64).clamp(0, 100));

    // Window title (shown in taskbar for borderless window)
    let _ = mpv.set_property("title", format!("Media Player — {}", title_name).as_str());

    // Initial window size — use percentage to respect display scaling on HiDPI screens
    let _ = mpv.set_property("autofit", "55%x55%");
    let _ = mpv.set_property("autofit-larger", "90%x90%");
    let _ = mpv.set_property("hidpi-window-scale", true);

    // Prevent mpv from auto-resizing the window when the d3d11vpp (RTX VSR)
    // filter changes video-out dimensions. Without this, enabling VSR in
    // fullscreen then exiting causes the window to shrink to near-zero.
    let _ = mpv.set_property("auto-window-resize", false);

    // Load and play the file
    let path_str = mpv_path_string(&path);

    // Audio visualization: showwaves renders a real-time white waveform on
    // black background.  format=pix_fmts=rgb24 strips the alpha channel.
    let is_audio = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(crate::infrastructure::windows::is_audio_extension)
        .unwrap_or(false);
    if is_audio {
        let _ = mpv.set_property(
            "lavfi-complex",
            "[aid1]asplit[ao][a1];[a1]showwaves=s=1920x1080:mode=cline:rate=30:colors=white,format=pix_fmts=rgb24[vo]",
        );
    }

    if let Err(e) = mpv.command("loadfile", &[&path_str]) {
        log::error!("[VIDEO-PLAYER] Failed to load file '{}': {:?}", path_str, e);
        return Ok(());
    }

    log::info!(
        "[VIDEO-PLAYER] Playing '{}' (pos={:.1}s, vol={:.0}%)",
        title_name,
        position,
        volume * 100.0
    );

    // Event loop — blocks until mpv shuts down (user closes window or presses 'q')
    let mut seek_applied = false;
    let mut eof_reached = false;
    loop {
        let event = mpv.wait_event(1.0);
        // Log every non-None event at debug level so we can trace what fires
        // when the close button is clicked at EOF (keep-open=yes paused state).
        if let Some(ref ev) = event {
            log::debug!("[VIDEO-PLAYER] event: {:?}", ev);
        }
        match event {
            Some(Ok(mpv::events::Event::Shutdown)) => {
                log::debug!("[VIDEO-PLAYER] mpv shutdown event received");
                break;
            }
            Some(Ok(mpv::events::Event::FileLoaded)) => {
                // Set our app icon on the mpv window (replaces default mpv icon)
                #[cfg(target_os = "windows")]
                set_mpv_window_icon(&mpv);

                // Log effective GPU pipeline for VSR debugging
                let vo = mpv.get_property::<String>("vo").unwrap_or_default();
                let gpu_api = mpv.get_property::<String>("gpu-api").unwrap_or_default();
                let gpu_ctx = mpv.get_property::<String>("gpu-context").unwrap_or_default();
                let hwdec_pref = mpv.get_property::<String>("hwdec").unwrap_or_default();
                let hwdec = mpv.get_property::<String>("hwdec-current").unwrap_or_default();
                log::info!(
                    "[VIDEO-PLAYER] Pipeline: vo={}, gpu-api={}, gpu-context={}, hwdec={}, hwdec-current={}",
                    vo, gpu_api, gpu_ctx, hwdec_pref, hwdec
                );

                // Apply initial seek position once file is loaded
                if !seek_applied && position > 0.5 {
                    let _ = mpv.set_property("time-pos", position);
                    seek_applied = true;
                    log::debug!("[VIDEO-PLAYER] Seeked to {:.1}s", position);
                }
            }
            Some(Ok(mpv::events::Event::ClientMessage(args))) => {
                if args.first() == Some(&"open-subtitle-picker") {
                    match load_external_subtitle_for_standalone(&mut mpv, &path) {
                        Ok(true) => {
                            log::debug!("[VIDEO-PLAYER] External subtitle loaded from native picker");
                        }
                        Ok(false) => {
                            let cancelled_msg = rust_i18n::t!("video.subtitle_cancelled").to_string();
                            let _ = mpv.command("show-text", &[&cancelled_msg, "1500"]);
                        }
                        Err(err) => {
                            log::warn!("[VIDEO-PLAYER] Failed to load subtitle from native picker: {}", err);
                            let _ = mpv.command("show-text", &[&err, "3000"]);
                        }
                    }
                }
            }
            Some(Ok(mpv::events::Event::EndFile(reason))) => {
                log::debug!("[VIDEO-PLAYER] EndFile reason={}", reason);

                // MPV_END_FILE_REASON constants: EOF=0, STOP=2, QUIT=3, ERROR=4, REDIRECT=5
                const REASON_EOF:  u32 = mpv::mpv_end_file_reason::Eof;
                const REASON_STOP: u32 = mpv::mpv_end_file_reason::Stop;
                const REASON_QUIT: u32 = mpv::mpv_end_file_reason::Quit;

                match reason {
                    REASON_EOF => {
                        // keep-open=yes: video reached end, player stays open showing
                        // last frame. Mark flag so we know we're in paused-at-EOF state.
                        eof_reached = true;
                        log::debug!("[VIDEO-PLAYER] EOF reached — keep-open holds player open");
                    }
                    REASON_STOP => {
                        // Stop fires when navigating playlist (playlist-play-index)
                        // or when the OSC triggers a file change. Do NOT exit here;
                        // the real quit path goes through Shutdown event.
                        // Reset flags for the next file.
                        eof_reached = false;
                        seek_applied = true; // don't re-seek on playlist navigation
                        log::debug!("[VIDEO-PLAYER] EndFile Stop — playlist navigation or file change");
                    }
                    REASON_QUIT => {
                        log::debug!("[VIDEO-PLAYER] EndFile Quit — exiting");
                        break;
                    }
                    _ => {
                        if eof_reached {
                            log::info!(
                                "[VIDEO-PLAYER] EndFile reason={} after EOF — exiting",
                                reason
                            );
                            break;
                        }
                    }
                }
            }
            Some(Err(e)) => {
                log::warn!("[VIDEO-PLAYER] mpv event error: {:?}", e);
            }
            _ => {}
        }
    }

    log::debug!("[VIDEO-PLAYER] Exiting standalone player");
    Ok(())
}
