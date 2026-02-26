//! Standalone dedicated video player mode (separate process).
//!
//! When the user clicks "detach" on the docked video player, the main app
//! spawns a new process (`--video-player <path> [--position <secs>] [--volume <vol>]`)
//! that runs an independent eframe window with MPV rendering.
//!
//! This follows the same pattern as `image_viewer::open_image_viewer()`.

mod app;

use std::path::PathBuf;
use std::process::{Child, Command};

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

/// Entry point for the standalone video player process.
pub fn run_standalone(path: PathBuf, position: f64, volume: f32) -> eframe::Result<()> {
    let title_name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "Video Player".to_string());

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(format!("Video Player — {}", title_name))
        .with_inner_size([960.0, 540.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-video-player");

    // Set window icon if possible
    if let Ok(img) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
        let rgba_image = resized.to_rgba8();
        viewport = viewport.with_icon(eframe::egui::IconData {
            rgba: rgba_image.into_raw(),
            width: 256,
            height: 256,
        });
    }

    let options = eframe::NativeOptions {
        viewport,
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "Video Player",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(app::DedicatedVideoPlayerApp::new(
                path, position, volume,
            )))
        }),
    )
}
