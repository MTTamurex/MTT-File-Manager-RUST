//! Native text-file viewer — renders plain text with monospace font.
//!
//! Spawns a dedicated process (same executable with `--text-viewer` flag)
//! so the viewer is fully independent from the main file-manager window.
//!
//! ## Security
//!
//! - Path validation: blocks UNC paths, null bytes, path traversal (`..`),
//!   and non-text extensions before any file I/O.
//! - File size limit: rejects files larger than [`MAX_TEXT_FILE_SIZE`].
//! - Binary detection: rejects files with too many null bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::viewer_runtime::{apply_saved_locale, build_viewer_native_options, is_saved_theme_dark};

mod viewer_app;

/// Maximum text file size accepted by the viewer (50 MB).
const MAX_TEXT_FILE_SIZE: u64 = 25 * 1024 * 1024;

/// Known text file extensions (lowercase, without dot).
const TEXT_EXTENSIONS: &[&str] = &[
    // Plain text / logs
    "txt", "log", "csv", "tsv", "nfo", "diz",
    // Config
    "cfg", "conf", "ini", "env", "properties", "toml", "yaml", "yml",
    "editorconfig", "gitignore", "gitattributes", "dockerignore",
    // Data / markup
    "json", "xml", "svg", "html", "htm", "css", "scss", "sass", "less",
    // Code
    "rs", "py", "js", "ts", "jsx", "tsx", "c", "cpp", "h", "hpp",
    "cs", "java", "go", "rb", "php", "swift", "kt", "kts", "scala",
    "lua", "r", "m", "mm", "pl", "pm", "sql",
    // Shell / scripting
    "sh", "bash", "zsh", "fish", "bat", "cmd", "ps1", "psm1", "psd1",
    // Documentation
    "md", "markdown", "rst", "tex", "adoc",
];

/// Check whether the given extension (without dot, case-insensitive)
/// corresponds to a known text file type.
pub fn is_text_extension(ext: &str) -> bool {
    let lower = ext.to_ascii_lowercase();
    TEXT_EXTENSIONS.iter().any(|&e| e == lower)
}

// ── Path validation ───────────────────────────────────────────────────────────

fn validate_text_path(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    // 1. Null bytes
    if path_str.contains('\0') {
        return Err(rust_i18n::t!("textviewer.invalid_null").to_string());
    }

    // 2. Path traversal
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(
                rust_i18n::t!(
                    "textviewer.invalid_traversal",
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
        return Err(rust_i18n::t!("textviewer.network_path").to_string());
    }

    // 4. Extension check
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if !is_text_extension(ext) {
        return Err(rust_i18n::t!("textviewer.invalid_extension", ext = ext).to_string());
    }

    // 5. File existence
    if !path.is_file() {
        return Err(
            rust_i18n::t!("textviewer.file_not_found", path = path.display().to_string())
                .to_string(),
        );
    }

    // 6. File size
    match std::fs::metadata(path) {
        Ok(meta) => {
            if meta.len() > MAX_TEXT_FILE_SIZE {
                return Err(
                    rust_i18n::t!(
                        "textviewer.file_too_large",
                        size_mb = format!("{:.1}", meta.len() as f64 / (1024.0 * 1024.0)),
                        max_mb = (MAX_TEXT_FILE_SIZE / (1024 * 1024)).to_string()
                    )
                    .to_string(),
                );
            }
        }
        Err(e) => {
            return Err(
                rust_i18n::t!("textviewer.metadata_read_failed", error = e.to_string())
                    .to_string(),
            )
        }
    }

    Ok(())
}

/// Open a text file in a new standalone viewer process (fire-and-forget).
pub fn open_text_viewer(path: PathBuf) {
    if let Err(e) = validate_text_path(&path) {
        log::error!("[TEXT-VIEWER] path validation failed for '{}': {}", path.display(), e);
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(v) => v,
        Err(err) => {
            log::error!(
                "[TEXT-VIEWER] failed to locate current executable for spawn: {}",
                err
            );
            return;
        }
    };

    if let Err(err) = Command::new(exe).arg("--text-viewer").arg(&path).spawn() {
        log::error!(
            "[TEXT-VIEWER] failed to spawn viewer for '{}': {}",
            path.display(),
            err
        );
    }
}

/// Entry-point called when the process is started with `--text-viewer <path>`.
pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    apply_saved_locale();

    if let Err(e) = validate_text_path(&path) {
        log::error!("[TEXT-VIEWER] path validation failed: {}", e);
        return show_error_window(&e);
    }

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| rust_i18n::t!("textviewer.title").to_string());

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(rust_i18n::t!("textviewer.title_with_file", name = file_name).to_string())
        .with_inner_size([1024.0, 768.0])
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-text-viewer");

    if let Ok(img) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
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
        &rust_i18n::t!("textviewer.title"),
        options,
        Box::new(move |_cc| {
            match viewer_app::TextViewerApp::new(path, dark_mode) {
                Ok(app) => Ok(Box::new(app)),
                Err(e) => {
                    log::error!("[TEXT-VIEWER] failed to open text file: {}", e);
                    Ok(Box::new(viewer_app::ErrorApp { message: e }))
                }
            }
        }),
    )
}

/// Show an error message in a minimal eframe window and exit.
fn show_error_window(message: &str) -> eframe::Result<()> {
    let msg = message.to_string();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(rust_i18n::t!("textviewer.title_error").to_string())
            .with_inner_size([500.0, 200.0]),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        &rust_i18n::t!("textviewer.app_error"),
        options,
        Box::new(move |_cc| Ok(Box::new(viewer_app::ErrorApp { message: msg }))),
    )
}
