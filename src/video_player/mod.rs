//! Standalone dedicated video player mode (separate process).
//!
//! When the user clicks "detach" on the docked video player, the main app
//! spawns a new process (`--video-player <path> [--position <secs>] [--volume <vol>]`)
//! that runs an independent mpv window (borderless, with OSC window controls).
//!
//! mpv creates its own native window (no `wid` embedding), so all native features
//! work: keyboard shortcuts, OSC, window management via OSC buttons.

use std::path::PathBuf;
use std::process::{Child, Command};

/// OSC script-opts for the standalone player.
/// - scalewindowed/scalefullscreen: OSC element sizing (1.0 = default)
/// - windowcontrols=yes: always show close/minimize/maximize in OSC
const STANDALONE_OSC_SCRIPT_OPTS: &str =
    "osc-scalewindowed=2,osc-scalefullscreen=2,osc-windowcontrols=yes";

/// Spawn a standalone video player process for the given file.
///
/// Returns the `Child` handle so the caller can track/kill the process.
pub fn open_video_player(path: PathBuf, position: f64, volume: f32) -> Option<Child> {
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
        .find(|dir| {
            let scripts = dir.join("scripts");
            scripts.join("osc.lua").is_file() && scripts.join("vsr.lua").is_file()
        })
}

/// Convert a Windows path to forward-slash form for mpv options.
fn mpv_path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Entry point for the standalone video player process.
///
/// Creates a native mpv window (borderless) with the custom OSC providing
/// window controls (close, minimize, maximize). No eframe wrapper needed.
pub fn run_standalone(path: PathBuf, position: f64, volume: f32) -> eframe::Result<()> {
    let title_name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "Video Player".to_string());

    let config_dir = resolve_mpv_ui_config_dir();
    if let Some(dir) = &config_dir {
        log::info!(
            "[VIDEO-PLAYER] Using mpv config dir: {}",
            dir.to_string_lossy()
        );
    } else {
        log::warn!(
            "[VIDEO-PLAYER] mpv_ui/portable_config not found (with scripts/osc.lua + scripts/vsr.lua); OSC/VSR may not load"
        );
    }

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
        if let Err(e) = init.set_option("script-opts", STANDALONE_OSC_SCRIPT_OPTS) {
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
    let _ = mpv.set_property("video-sync", "audio");
    let _ = mpv.set_property("interpolation", false);
    let _ = mpv.set_property("tscale", "linear");
    let _ = mpv.set_property("framedrop", "vo");
    let _ = mpv.set_property("keep-open", "yes");

    // Cache/demuxer settings
    let _ = mpv.set_property("cache", "yes");
    let _ = mpv.set_property("cache-secs", 12.0_f64);
    let _ = mpv.set_property("demuxer-readahead-secs", 6.0_f64);
    let _ = mpv.set_property("demuxer-max-bytes", 48_i64 * 1024 * 1024);
    let _ = mpv.set_property("demuxer-max-back-bytes", 12_i64 * 1024 * 1024);

    // Volume (mpv uses 0-100 scale)
    let _ = mpv.set_property("volume", ((volume * 100.0) as i64).clamp(0, 100));

    // Window title (shown in taskbar for borderless window)
    let _ = mpv.set_property("title", format!("Video Player — {}", title_name).as_str());

    // Initial window size — use percentage to respect display scaling on HiDPI screens
    let _ = mpv.set_property("autofit", "55%x55%");
    let _ = mpv.set_property("autofit-larger", "90%x90%");
    let _ = mpv.set_property("hidpi-window-scale", true);

    // Load and play the file
    let path_str = mpv_path_string(&path);
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
                log::info!("[VIDEO-PLAYER] mpv shutdown event received");
                break;
            }
            Some(Ok(mpv::events::Event::FileLoaded)) => {
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
                    log::info!("[VIDEO-PLAYER] Seeked to {:.1}s", position);
                }
            }
            Some(Ok(mpv::events::Event::EndFile(reason))) => {
                log::info!("[VIDEO-PLAYER] EndFile reason={}", reason);

                // MPV_END_FILE_REASON constants: EOF=0, STOP=2, QUIT=3, ERROR=4, REDIRECT=5
                const REASON_EOF:  u32 = mpv::mpv_end_file_reason::Eof;
                const REASON_STOP: u32 = mpv::mpv_end_file_reason::Stop;
                const REASON_QUIT: u32 = mpv::mpv_end_file_reason::Quit;

                match reason {
                    REASON_EOF => {
                        // keep-open=yes: video reached end, player stays open showing
                        // last frame. Mark flag so we know we're in paused-at-EOF state.
                        eof_reached = true;
                        log::info!("[VIDEO-PLAYER] EOF reached — keep-open holds player open");
                    }
                    REASON_QUIT | REASON_STOP => {
                        // User explicitly closed the player (OSC close button, 'q' key,
                        // or equivalent). Exit the event loop.
                        log::info!("[VIDEO-PLAYER] EndFile Stop/Quit — exiting");
                        break;
                    }
                    _ => {
                        // ERROR or REDIRECT: only exit if we were already at EOF
                        // (i.e., the player had finished and something triggered close).
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

    log::info!("[VIDEO-PLAYER] Exiting standalone player");
    Ok(())
}
