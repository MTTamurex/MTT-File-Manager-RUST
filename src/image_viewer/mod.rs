use eframe::egui;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

mod app;
mod cache;
mod indexer;
mod ipc;
mod loader;
mod thumbnail_cache;
pub(crate) mod metrics;

use crate::viewer_runtime::{apply_saved_locale, build_viewer_native_options, is_saved_theme_dark};

const IMAGE_VIEWER_MUTEX_NAME: &str = "Local\\MTTFileManager_ImageViewer_SingleInstance\0";
const MAX_IMAGE_FILE_SIZE: u64 = 512 * 1024 * 1024;
const OPEN_REQUEST_DEBOUNCE: Duration = Duration::from_millis(700);

struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
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

fn recent_open_request_state() -> &'static Mutex<Option<(PathBuf, Instant)>> {
    static RECENT_OPEN_REQUEST: OnceLock<Mutex<Option<(PathBuf, Instant)>>> = OnceLock::new();
    RECENT_OPEN_REQUEST.get_or_init(|| Mutex::new(None))
}

fn paths_eq_case_insensitive(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

fn should_suppress_duplicate_open(path: &Path) -> bool {
    let Ok(mut state) = recent_open_request_state().lock() else {
        return false;
    };

    let now = Instant::now();
    let suppress = state
        .as_ref()
        .map(|(last_path, last_at)| {
            now.duration_since(*last_at) <= OPEN_REQUEST_DEBOUNCE
                && paths_eq_case_insensitive(last_path, path)
        })
        .unwrap_or(false);

    *state = Some((path.to_path_buf(), now));
    suppress
}

fn validate_image_path(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    if path_str.contains('\0') {
        return Err("Path contains null bytes".into());
    }

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

    if path_str.starts_with("\\\\")
        || path_str.starts_with("//")
        || path_str.starts_with("\\\\?\\UNC\\")
    {
        return Err("Network/UNC paths are not allowed".into());
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !crate::infrastructure::windows::is_image_extension(ext) {
        return Err(format!("Unsupported image extension: '{}'", ext));
    }

    if !path.is_file() {
        return Err(format!("File not found: '{}'", path.display()));
    }

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
    log::info!(
        "[IMAGE-VIEWER] open_image_viewer requested pid={} path='{}'",
        std::process::id(),
        path.display()
    );

    if should_suppress_duplicate_open(&path) {
        log::debug!(
            "[IMAGE-VIEWER] suppressing duplicate open request for '{}'",
            path.display()
        );
        return;
    }

    open_image_viewer_blocking(&path);
}

fn open_image_viewer_blocking(path: &Path) {
    if let Err(e) = validate_image_path(path) {
        log::error!(
            "[IMAGE-VIEWER] path validation failed for '{}': {}",
            path.display(),
            e
        );
        return;
    }

    match ipc::send_open_request(path) {
        Ok(true) => {
            log::info!(
                "[IMAGE-VIEWER] existing instance accepted open request pid={} path='{}'",
                std::process::id(),
                path.display()
            );
            return;
        }
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

    match Command::new(exe).arg("--image-viewer").arg(path).spawn() {
        Ok(child) => {
            log::info!(
                "[IMAGE-VIEWER] spawned standalone viewer parent_pid={} child_pid={} path='{}'",
                std::process::id(),
                child.id(),
                path.display()
            );
        }
        Err(err) => {
            log::error!(
                "[IMAGE-VIEWER] failed to spawn standalone viewer for '{}': {}",
                path.display(),
                err
            );
        }
    }
}

pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    // Capture panics so we can diagnose shutdown crashes even when stderr
    // is not attached (GUI-subsystem binary launched from shortcut).
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("[IMAGE-VIEWER] PANIC: {}", info);
        default_panic(info);
    }));

    log::info!(
        "[IMAGE-VIEWER] run_standalone enter pid={} path='{}'",
        std::process::id(),
        path.display()
    );

    if let Err(e) = validate_image_path(&path) {
        log::error!("[IMAGE-VIEWER] path validation failed in standalone: {}", e);
        return Ok(());
    }

    apply_saved_locale();

    // Remove any stale eframe storage (app.ron) written by previous runs.
    // eframe restores persisted window state (position, size, visibility)
    // before with_visible(false) takes effect, causing startup flicker.
    if let Some(mut p) = dirs::data_dir() {
        p.push("mtt-file-manager-image-viewer");
        p.push("data");
        p.push("app.ron");
        let _ = std::fs::remove_file(&p);
    }

    let _guard = match SingleInstanceGuard::try_acquire() {
        Some(g) => g,
        None => {
            match ipc::send_open_request(&path) {
                Ok(true) => {
                    log::info!("[IMAGE-VIEWER] forwarded image to existing viewer instance");
                }
                Ok(false) => {
                    log::warn!("[IMAGE-VIEWER] existing instance unavailable for IPC forward");
                }
                Err(err) => {
                    log::warn!(
                        "[IMAGE-VIEWER] failed to forward image to existing viewer: {}",
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
        (
            startup_sequence_rx,
            indexer::ImageSequence::single(path.clone()),
        )
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
        viewport = viewport.with_icon(egui::IconData {
            rgba: rgba_image.into_raw(),
            width: 256,
            height: 256,
        });
    }

    let native_options = build_viewer_native_options(viewport);
    let dark_mode = is_saved_theme_dark();

    let result = eframe::run_native(
        &rust_i18n::t!("imageviewer.title"),
        native_options,
        Box::new(move |_cc| {
            Ok(Box::new(app::DedicatedImageViewerApp::new(
                initial_sequence,
                external_open_rx,
                dark_mode,
                startup_sequence_rx,
                startup_preview,
            )))
        }),
    );

    // Force-exit to avoid hangs from detached background threads
    // (IPC server blocked in ConnectNamedPipe, GIF decode mid-flight,
    //  PrefetchEngine workers finishing a slow decode, etc.).
    // The main app uses the same belt-and-suspenders approach.
    #[cfg(target_os = "windows")]
    {
        let _ = std::thread::spawn(
            crate::infrastructure::windows::cancel_pending_io_on_current_process_threads,
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
        crate::infrastructure::windows::terminate_current_process(0);
    }

    result
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
