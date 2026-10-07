use std::path::{Path, PathBuf};
use std::process::Command;

use crate::viewer_runtime::{apply_saved_locale, build_viewer_native_options, is_saved_theme_dark};

mod render_worker;
mod renderer;
mod text_wrap;
mod viewer_app;

const MAX_XLSX_FILE_SIZE: u64 = 512 * 1024 * 1024;
const XLSX_VIEWER_LINE_SCROLL_SPEED: f32 = 120.0;

pub fn is_xlsx_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case("xlsx")
}

fn validate_xlsx_path(path: &Path) -> Result<(), String> {
    let path_string = path.to_string_lossy();
    if path_string.contains('\0') {
        return Err(rust_i18n::t!("xlsxviewer.invalid_null").to_string());
    }

    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(rust_i18n::t!("xlsxviewer.invalid_traversal").to_string());
        }
    }

    if path_string.starts_with("\\\\")
        || path_string.starts_with("//")
        || path_string.starts_with("\\\\?\\UNC\\")
    {
        return Err(rust_i18n::t!("xlsxviewer.network_path").to_string());
    }

    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !is_xlsx_extension(extension) {
        return Err(rust_i18n::t!("xlsxviewer.invalid_extension").to_string());
    }

    if !path.is_file() {
        return Err(rust_i18n::t!("xlsxviewer.file_not_found").to_string());
    }

    let metadata = std::fs::metadata(path).map_err(|error| {
        rust_i18n::t!("xlsxviewer.metadata_read_failed", error = error).to_string()
    })?;
    validate_xlsx_file_size(metadata.len())
}

fn validate_xlsx_file_size(size: u64) -> Result<(), String> {
    if size > MAX_XLSX_FILE_SIZE {
        return Err(rust_i18n::t!(
            "xlsxviewer.file_too_large",
            size_mb = format!("{:.1}", size as f64 / (1024.0 * 1024.0)),
            max_mb = (MAX_XLSX_FILE_SIZE / (1024 * 1024)).to_string()
        )
        .to_string());
    }
    Ok(())
}

pub fn open_xlsx_viewer(path: PathBuf) {
    if let Err(error) = validate_xlsx_path(&path) {
        log::error!("[XLSX-VIEWER] path validation failed: {error}");
        return;
    }

    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            log::error!("[XLSX-VIEWER] failed to locate executable: {error}");
            return;
        }
    };

    match Command::new(executable)
        .arg("--xlsx-viewer")
        .arg(path)
        .spawn()
    {
        Ok(child) => crate::viewer_processes::register(child),
        Err(error) => log::error!("[XLSX-VIEWER] failed to start viewer: {error}"),
    }
}

pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    apply_saved_locale();
    if let Err(error) = validate_xlsx_path(&path) {
        log::error!("[XLSX-VIEWER] path validation failed: {error}");
        return show_error_window(&error);
    }

    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| rust_i18n::t!("xlsxviewer.title").to_string());
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(rust_i18n::t!("xlsxviewer.title_with_file", name = file_name).to_string())
        .with_inner_size([1200.0, 820.0])
        .with_visible(false)
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-xlsx-viewer");

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
        &rust_i18n::t!("xlsxviewer.title"),
        options,
        Box::new(move |creation_context| {
            creation_context.egui_ctx.options_mut(|options| {
                options.input_options.line_scroll_speed = XLSX_VIEWER_LINE_SCROLL_SPEED;
            });
            Ok(Box::new(viewer_app::XlsxViewerApp::new(path, dark_mode)))
        }),
    )
}

fn show_error_window(message: &str) -> eframe::Result<()> {
    let message = message.to_owned();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(rust_i18n::t!("xlsxviewer.title_error").to_string())
            .with_inner_size([500.0, 200.0]),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        &rust_i18n::t!("xlsxviewer.app_error"),
        options,
        Box::new(move |_creation_context| Ok(Box::new(viewer_app::ErrorApp { message }))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xlsx_extension_check_is_case_insensitive() {
        assert!(is_xlsx_extension("xlsx"));
        assert!(is_xlsx_extension("XLSX"));
        assert!(!is_xlsx_extension("xls"));
        assert!(!is_xlsx_extension(""));
    }

    #[test]
    fn xlsx_path_validation_accepts_case_insensitive_extensions() {
        let file = tempfile::Builder::new().suffix(".XLSX").tempfile().unwrap();
        assert!(validate_xlsx_path(file.path()).is_ok());
    }

    #[test]
    fn xlsx_size_validation_enforces_the_configured_limit() {
        assert!(validate_xlsx_file_size(MAX_XLSX_FILE_SIZE).is_ok());
        assert!(validate_xlsx_file_size(MAX_XLSX_FILE_SIZE + 1).is_err());
    }
}
