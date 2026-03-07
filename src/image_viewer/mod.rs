use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

mod app;
mod cache;
mod ipc;
mod indexer;
mod loader;

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

pub fn open_image_viewer(path: PathBuf) {
    match ipc::send_open_request(&path) {
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

    let spawn_result = Command::new(exe)
        .arg("--image-viewer")
        .arg(&path)
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

    let sequence = match indexer::build_sequence(&path) {
        Ok(sequence) => sequence,
        Err(err) => {
            log::warn!(
                "[IMAGE-VIEWER] failed to build sequence for '{}': {}",
                path.display(),
                err
            );
            indexer::ImageSequence::single(path.clone())
        }
    };

    let start_index = sequence.current_index.min(sequence.entries.len().saturating_sub(1));
    let title_name = sequence
        .entries
        .get(start_index)
        .and_then(|p| p.file_name())
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "Image Viewer".to_string());

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(format!("Image Viewer - {}", title_name))
        .with_inner_size([1200.0, 850.0])
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

    let options = eframe::NativeOptions {
        viewport,
        persist_window: false,
        ..Default::default()
    };

    eframe::run_native(
        "Image Viewer",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(app::DedicatedImageViewerApp::new(
                sequence,
                external_open_rx,
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

