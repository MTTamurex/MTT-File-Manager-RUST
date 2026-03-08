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
        return Err(rust_i18n::t!("pdfviewer.invalid_null").to_string());
    }

    // 2. Path traversal
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(
                rust_i18n::t!(
                    "pdfviewer.invalid_traversal",
                    component = component.as_os_str().to_string_lossy().to_string()
                )
                .to_string(),
            );
        }
    }

    // 3. Block UNC / network paths
    if path_str.starts_with("\\\\")
        || path_str.starts_with("//")
        || path_str.starts_with("\\\\?\\UNC\\")
    {
        return Err(rust_i18n::t!("pdfviewer.network_path").to_string());
    }

    // 4. Extension check
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !ext.eq_ignore_ascii_case("pdf") {
        return Err(rust_i18n::t!("pdfviewer.invalid_extension", ext = ext).to_string());
    }

    // 5. File existence
    if !path.is_file() {
        return Err(
            rust_i18n::t!("pdfviewer.file_not_found", path = path.display().to_string())
                .to_string(),
        );
    }

    // 6. File size
    match std::fs::metadata(path) {
        Ok(meta) => {
            if meta.len() > MAX_PDF_FILE_SIZE {
                return Err(
                    rust_i18n::t!(
                        "pdfviewer.file_too_large",
                        size_mb = format!("{:.1}", meta.len() as f64 / (1024.0 * 1024.0)),
                        max_mb = (MAX_PDF_FILE_SIZE / (1024 * 1024)).to_string()
                    )
                    .to_string(),
                );
            }
        }
        Err(e) => {
            return Err(
                rust_i18n::t!("pdfviewer.metadata_read_failed", error = e.to_string())
                    .to_string(),
            )
        }
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
    apply_saved_locale();

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
        .unwrap_or_else(|| rust_i18n::t!("pdfviewer.title").to_string());

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(rust_i18n::t!("pdfviewer.title_with_file", name = file_name).to_string())
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
        &rust_i18n::t!("pdfviewer.title"),
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
            .with_title(rust_i18n::t!("pdfviewer.title_error").to_string())
            .with_inner_size([500.0, 200.0]),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        &rust_i18n::t!("pdfviewer.app_error"),
        options,
        Box::new(move |_cc| Ok(Box::new(viewer_app::ErrorApp { message: msg }))),
    )
}
