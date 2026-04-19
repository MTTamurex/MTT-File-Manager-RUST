use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

mod app;
mod cache;
mod ipc;
mod indexer;
mod loader;

use crate::viewer_runtime::{apply_saved_locale, build_viewer_native_options, is_saved_theme_dark};

/// Named mutex used to guarantee only one image viewer instance runs at a time.
const IMAGE_VIEWER_MUTEX_NAME: &str = "Global\\MTTFileManager_ImageViewer_SingleInstance\0";

/// RAII guard that holds the named mutex for the viewer's lifetime.
struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    /// Returns `Some(guard)` if this is the first viewer instance.
    /// Returns `None` if another viewer instance already owns the mutex.
    fn try_acquire() -> Option<Self> {
        let wide: Vec<u16> = IMAGE_VIEWER_MUTEX_NAME.encode_utf16().collect();
        unsafe {
            let handle = CreateMutexW(None, true, PCWSTR(wide.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                None
            } else {
                Some(Self { handle })
            }
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Maximum file size for the image viewer (512 MB).
const MAX_IMAGE_FILE_SIZE: u64 = 512 * 1024 * 1024;

/// SEC: Validate the image path before opening. Blocks null bytes, path traversal,
/// UNC/network paths, non-image extensions, and oversized files.
fn validate_image_path(path: &Path) -> Result<(), String> {
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

    // 4. Extension whitelist
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !crate::infrastructure::windows::is_image_extension(ext) {
        return Err(format!("Unsupported image extension: '{}'", ext));
    }

    // 5. File existence
    if !path.is_file() {
        return Err(format!("File not found: '{}'", path.display()));
    }

    // 6. File size
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_IMAGE_FILE_SIZE {
            return Err(format!(
                "File too large: {:.1} MB (max {} MB)",
                meta.len() as f64 / (1024.0 * 1024.0),
                MAX_IMAGE_FILE_SIZE / (1024 * 1024)
            ));
        }
    }

    Ok(())
}

pub fn open_image_viewer(path: PathBuf) {
    // Spawn a thread so blocking I/O (validation, IPC retry) does not stall the UI.
    if let Err(err) = std::thread::Builder::new()
        .name("open-image-viewer".into())
        .spawn(move || {
            open_image_viewer_blocking(&path);
        })
    {
        log::error!("[IMAGE-VIEWER] failed to spawn open-image-viewer thread: {}", err);
    }
}

fn open_image_viewer_blocking(path: &Path) {
    // SEC: Validate path before spawning child process.
    if let Err(e) = validate_image_path(path) {
        log::error!("[IMAGE-VIEWER] path validation failed for '{}': {}", path.display(), e);
        return;
    }

    match ipc::send_open_request(path) {
        Ok(true) => return,
        Ok(false) => {}
        Err(err) => {
            log::warn!(
                "[IMAGE-VIEWER] failed to forward open request to existing instance: {}",
                err
            );
        }
    }

    let exe = match std::env::current_exe() {
        Ok(v) => v,
        Err(err) => {
            log::error!(
                "[IMAGE-VIEWER] failed to locate current executable for spawn: {}",
                err
            );
            return;
        }
    };

    // CREATE_NO_WINDOW prevents a transient console-window flash on Windows
    // (especially in debug builds where windows_subsystem is not set).
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let spawn_result = Command::new(exe)
        .arg("--image-viewer")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();

    if let Err(err) = spawn_result {
        log::error!(
            "[IMAGE-VIEWER] failed to spawn standalone viewer for '{}': {}",
            path.display(),
            err
        );
    }
}

pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    // SEC: Validate again in child process (defense in depth).
    if let Err(e) = validate_image_path(&path) {
        log::error!("[IMAGE-VIEWER] path validation failed in standalone: {}", e);
        return Ok(());
    }

    apply_saved_locale();

    let _guard = match SingleInstanceGuard::try_acquire() {
        Some(g) => g,
        None => {
            match ipc::send_open_request(&path) {
                Ok(true) => {
                    log::info!(
                        "[IMAGE-VIEWER] forwarded image to the existing viewer instance"
                    );
                }
                Ok(false) => {
                    log::warn!(
                        "[IMAGE-VIEWER] another instance exists, but its IPC server was unavailable"
                    );
                }
                Err(err) => {
                    log::warn!(
                        "[IMAGE-VIEWER] failed to forward image to the existing viewer: {}",
                        err
                    );
                }
            }
            return Ok(());
        }
    };

    let external_open_rx = ipc::start_open_request_server();

    let title_name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| rust_i18n::t!("imageviewer.title").to_string());

    // Do not start the viewer from the thumbnail cache. That path can return
    // a reduced preview image, which makes the first frame open with the wrong
    // apparent zoom and, because it occupied `current_index` in the cache,
    // could block the real full-frame decode from ever being requested.
    let startup_preview = None;

    let (startup_sequence_rx, initial_sequence) = {
        let (tx, rx) = std::sync::mpsc::channel();
        let path_clone = path.clone();

        let startup_sequence_rx = match std::thread::Builder::new()
            .name("image-viewer-startup-seq".into())
            .spawn(move || {
                let sequence = match indexer::build_sequence(&path_clone) {
                    Ok(sequence) => sequence,
                    Err(err) => {
                        log::warn!(
                            "[IMAGE-VIEWER] failed to build startup sequence for '{}': {}",
                            path_clone.display(),
                            err
                        );
                        indexer::ImageSequence::single(path_clone)
                    }
                };

                let _ = tx.send(sequence);
            }) {
            Ok(_) => Some(rx),
            Err(err) => {
                log::warn!(
                    "[IMAGE-VIEWER] failed to spawn startup sequence builder for '{}': {}",
                    path.display(),
                    err
                );
                None
            }
        };

        (startup_sequence_rx, indexer::ImageSequence::single(path.clone()))
    };

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(rust_i18n::t!("imageviewer.title_with_file", name = title_name).to_string())
        .with_inner_size([1200.0, 850.0])
        .with_visible(false)
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-image-viewer");

    if let Ok(img) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
        let rgba_image = resized.to_rgba8();
        viewport = viewport.with_icon(eframe::egui::IconData {
            rgba: rgba_image.into_raw(),
            width: 256,
            height: 256,
        });
    }

    let options = build_viewer_native_options(viewport);

    let dark_mode = is_saved_theme_dark();

    eframe::run_native(
        &rust_i18n::t!("imageviewer.title"),
        options,
        Box::new(move |_cc| {
            Ok(Box::new(app::DedicatedImageViewerApp::new(
                initial_sequence,
                external_open_rx,
                dark_mode,
                startup_sequence_rx,
                startup_preview,
            )))
        }),
    )
}

pub fn decode_full_for_benchmark(path: &Path) -> std::io::Result<(u32, u32, usize)> {
    let frame = loader::decode_full_frame(path)?;
    Ok((frame.width, frame.height, frame.rgba.len()))
}

pub fn decode_preview_for_benchmark(
    path: &Path,
    max_side: u32,
) -> std::io::Result<(u32, u32, usize)> {
    let frame = loader::decode_preview_frame(path, max_side)?;
    Ok((frame.width, frame.height, frame.rgba.len()))
}

