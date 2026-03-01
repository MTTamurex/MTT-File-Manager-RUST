//! Native PDF viewer — renders via the built-in Windows.Data.Pdf API.
//!
//! Spawns a dedicated process (same executable with `--pdf-viewer` flag)
//! so the viewer is fully independent from the main file-manager window.
//!
//! ## Security
//!
//! - Path validation: blocks UNC paths, null bytes, path traversal (`..`),
//!   and non-`.pdf` extensions before any file I/O.
//! - File size limit: rejects files larger than [`MAX_PDF_FILE_SIZE`].
//! - Memory budget: texture cache is bounded by [`viewer_app::TEXTURE_MEMORY_BUDGET`];
//!   oldest/furthest pages are evicted when exceeded.

use std::path::{Path, PathBuf};
use std::process::Command;

mod render_worker;
mod renderer;
mod toolbar;
mod viewer_app;

/// Maximum PDF file size accepted by the viewer (512 MB).
const MAX_PDF_FILE_SIZE: u64 = 512 * 1024 * 1024;

// ── Path validation ───────────────────────────────────────────────────────────────

/// Validate and sanitize the PDF path before any file I/O.
///
/// Checks:
/// 1. No null bytes
/// 2. No path traversal components (`..`, `.`)
/// 3. Not a UNC/network path
/// 4. Extension is `.pdf` (case-insensitive)
/// 5. File exists and is a regular file
/// 6. File size does not exceed [`MAX_PDF_FILE_SIZE`]
fn validate_pdf_path(path: &Path) -> Result<(), String> {
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
                "Path traversal component detected: {}",
                component.as_os_str().to_string_lossy()
            ));
        }
    }

    // 3. Block UNC / network paths
    if path_str.starts_with("\\\\")
        || path_str.starts_with("//")
        || path_str.starts_with("\\\\?\\UNC\\")
    {
        return Err("Network/UNC paths are not allowed for security".into());
    }

    // 4. Extension check
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !ext.eq_ignore_ascii_case("pdf") {
        return Err(format!(
            "Only .pdf files are accepted (got .{})",
            ext
        ));
    }

    // 5. File existence
    if !path.is_file() {
        return Err(format!("File not found: {}", path.display()));
    }

    // 6. File size
    match std::fs::metadata(path) {
        Ok(meta) => {
            if meta.len() > MAX_PDF_FILE_SIZE {
                return Err(format!(
                    "File too large ({:.1} MB). Maximum allowed: {} MB",
                    meta.len() as f64 / (1024.0 * 1024.0),
                    MAX_PDF_FILE_SIZE / (1024 * 1024)
                ));
            }
        }
        Err(e) => return Err(format!("Cannot read file metadata: {e}")),
    }

    Ok(())
}

/// Open a PDF in a new standalone viewer process (fire-and-forget).
pub fn open_pdf_viewer(path: PathBuf) {
    // Validate the path before spawning a child process.
    if let Err(e) = validate_pdf_path(&path) {
        log::error!("[PDF-VIEWER] path validation failed for '{}': {}", path.display(), e);
        return;
    }

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
    // Validate path again in the child process (defense in depth — the child
    // receives the path from the command line which could be tampered with).
    if let Err(e) = validate_pdf_path(&path) {
        log::error!("[PDF-VIEWER] path validation failed: {}", e);
        // Show error in a GUI window instead of silently exiting
        return show_error_window(&e);
    }

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

/// Show an error message in a minimal eframe window and exit.
fn show_error_window(message: &str) -> eframe::Result<()> {
    let msg = message.to_string();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("PDF Viewer - Error")
            .with_inner_size([500.0, 200.0]),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "PDF Viewer Error",
        options,
        Box::new(move |_cc| Ok(Box::new(viewer_app::ErrorApp { message: msg }))),
    )
}
