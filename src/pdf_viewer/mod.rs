//! Native PDF viewer — renders via the built-in Windows.Data.Pdf API.
//!
//! Spawns a dedicated process (same executable with `--pdf-viewer` flag)
//! so the viewer is fully independent from the main file-manager window.

use std::path::PathBuf;
use std::process::Command;

mod render_worker;
mod renderer;
mod toolbar;
mod viewer_app;

/// Open a PDF in a new standalone viewer process (fire-and-forget).
pub fn open_pdf_viewer(path: PathBuf) {
    let exe = match std::env::current_exe() {
        Ok(v) => v,
        Err(err) => {
            log::error!(
                "[PDF-VIEWER] failed to locate current executable for spawn: {}",
                err
            );
            return;
        }
    };

    if let Err(err) = Command::new(exe).arg("--pdf-viewer").arg(&path).spawn() {
        log::error!(
            "[PDF-VIEWER] failed to spawn viewer for '{}': {}",
            path.display(),
            err
        );
    }
}

/// Entry-point called when the process is started with `--pdf-viewer <path>`.
pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "PDF".to_string());

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(format!("PDF Viewer - {}", file_name))
        .with_inner_size([1024.0, 768.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-pdf-viewer");

    if let Ok(img) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
        let rgba = resized.to_rgba8();
        viewport = viewport.with_icon(eframe::egui::IconData {
            rgba: rgba.into_raw(),
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
        "PDF Viewer",
        options,
        Box::new(move |_cc| match viewer_app::PdfViewerApp::new(path) {
            Ok(app) => Ok(Box::new(app)),
            Err(e) => {
                log::error!("[PDF-VIEWER] failed to open PDF: {}", e);
                Ok(Box::new(viewer_app::ErrorApp { message: e }))
            }
        }),
    )
}

/// No-op — WebView2 warmup is no longer needed.
pub fn warmup() {}
