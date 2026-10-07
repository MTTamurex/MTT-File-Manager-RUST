//! Read-only DOCX viewer powered by BetterOffice's native parsing, layout and raster crates.
//!
//! The viewer runs in a separate process (`--docx-viewer`) like the PDF and text viewers. The
//! parse/layout/raster work is isolated on a background thread inside that process.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::viewer_runtime::{apply_saved_locale, build_viewer_native_options, is_saved_theme_dark};

mod fonts;
mod images;
mod raster_scale;
mod render_worker;
mod renderer;
mod search;
mod viewer_app;

const MAX_DOCX_FILE_SIZE: u64 = 512 * 1024 * 1024;
const DOCX_VIEWER_LINE_SCROLL_SPEED: f32 = 120.0;

pub fn is_docx_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("docx")
}

fn validate_docx_path(path: &Path) -> Result<(), String> {
    let path_string = path.to_string_lossy();
    if path_string.contains('\0') {
        return Err(rust_i18n::t!("docxviewer.invalid_null").to_string());
    }

    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(rust_i18n::t!("docxviewer.invalid_traversal").to_string());
        }
    }

    if path_string.starts_with("\\\\")
        || path_string.starts_with("//")
        || path_string.starts_with("\\\\?\\UNC\\")
    {
        return Err(rust_i18n::t!("docxviewer.network_path").to_string());
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !is_docx_extension(extension) {
        return Err(rust_i18n::t!("docxviewer.invalid_extension").to_string());
    }

    if !path.is_file() {
        return Err(rust_i18n::t!("docxviewer.file_not_found").to_string());
    }

    let metadata = std::fs::metadata(path).map_err(|error| {
        rust_i18n::t!("docxviewer.metadata_read_failed", error = error).to_string()
    })?;
    if metadata.len() > MAX_DOCX_FILE_SIZE {
        return Err(rust_i18n::t!(
            "docxviewer.file_too_large",
            size_mb = format!("{:.1}", metadata.len() as f64 / (1024.0 * 1024.0)),
            max_mb = (MAX_DOCX_FILE_SIZE / (1024 * 1024)).to_string()
        )
        .to_string());
    }

    Ok(())
}

/// Launch a DOCX in a standalone, read-only viewer process.
pub fn open_docx_viewer(path: PathBuf) {
    if let Err(error) = validate_docx_path(&path) {
        log::error!("[DOCX-VIEWER] path validation failed: {error}");
        return;
    }

    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            log::error!("[DOCX-VIEWER] failed to locate executable: {error}");
            return;
        }
    };

    match Command::new(executable)
        .arg("--docx-viewer")
        .arg(path)
        .spawn()
    {
        Ok(child) => crate::viewer_processes::register(child),
        Err(error) => log::error!("[DOCX-VIEWER] failed to start viewer: {error}"),
    }
}

/// Entry point for `--docx-viewer <path>`.
pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    apply_saved_locale();
    if let Err(error) = validate_docx_path(&path) {
        log::error!("[DOCX-VIEWER] path validation failed: {error}");
        return show_error_window(&error);
    }

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| rust_i18n::t!("docxviewer.title").to_string());
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(rust_i18n::t!("docxviewer.title_with_file", name = file_name).to_string())
        .with_inner_size([1100.0, 820.0])
        .with_visible(false)
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-docx-viewer");

    if let Ok(image) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = image.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
        let rgba = resized.to_rgba8();
        viewport = viewport.with_icon(eframe::egui::IconData {
            rgba: rgba.into_raw(),
            width: 256,
            height: 256,
        });
    }

    let options = build_viewer_native_options(viewport);
    let dark_mode = is_saved_theme_dark();
    eframe::run_native(
        &rust_i18n::t!("docxviewer.title"),
        options,
        Box::new(move |creation_context| {
            creation_context.egui_ctx.options_mut(|options| {
                options.input_options.line_scroll_speed = DOCX_VIEWER_LINE_SCROLL_SPEED;
            });
            Ok(Box::new(viewer_app::DocxViewerApp::new(path, dark_mode)))
        }),
    )
}

fn show_error_window(message: &str) -> eframe::Result<()> {
    let message = message.to_owned();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(rust_i18n::t!("docxviewer.title_error").to_string())
            .with_inner_size([500.0, 200.0]),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        &rust_i18n::t!("docxviewer.app_error"),
        options,
        Box::new(move |_creation_context| Ok(Box::new(viewer_app::ErrorApp { message }))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_extension_check_is_case_insensitive() {
        assert!(is_docx_extension("docx"));
        assert!(is_docx_extension("DOCX"));
        assert!(!is_docx_extension("doc"));
        assert!(!is_docx_extension(""));
    }
}
