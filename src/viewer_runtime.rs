//! Shared lightweight runtime helpers for the standalone viewer subprocesses
//! (`--image-viewer`, `--pdf-viewer`, `--text-viewer`).
//!
//! The viewers run as separate processes spawned from the same binary as the
//! main file manager, so without care each one would inherit the file
//! manager's heavy startup cost (full SQLite ORM init, all wgpu backends,
//! discrete-GPU device, performance memory hints, etc.) just to display a
//! single file. This module concentrates the minimal-baseline configuration:
//!
//! * Read user prefs (locale, theme) with a tiny read-only SQLite query
//!   instead of the full [`crate::infrastructure::app_state_db::AppStateDb`]
//!   pipeline (which runs migrations and pragmas on every open).
//! * Build [`eframe::NativeOptions`] that
//!   - use the lighter `Glow` renderer instead of `Wgpu`; for the standalone
//!     viewers this avoids the large DX12 / wgpu baseline that dominates RSS
//!     even for tiny text files;
//!   - disable optional GL-only buffers (`depth_buffer`, `stencil_buffer`,
//!     `multisampling`) which the viewers never use.

use eframe::egui;
use std::path::PathBuf;

fn state_db_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("MTT-File-Manager")
            .join("state")
            .join("app_state.db"),
    )
}

/// Reads a single value from the `user_preferences` table using a lightweight
/// read-only SQLite connection. Avoids running the full
/// [`crate::infrastructure::app_state_db::AppStateDb`] init (migrations,
/// pragmas, prepared-statement cache) which on cold start can cost tens of
/// MB of resident memory in each viewer process.
fn read_pref_readonly(key: &str) -> Option<String> {
    let db_path = state_db_path()?;
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM user_preferences WHERE key = ?",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .ok();
    // Connection drops here, releasing any cached pages.
    value
}

/// Apply the saved UI language to `rust_i18n` for this viewer process.
pub fn apply_saved_locale() {
    if let Some(language) = read_pref_readonly("language") {
        rust_i18n::set_locale(&language);
    }
}

/// Returns `true` if the user's saved theme is "dark", `false` otherwise
/// (including when no preference is stored).
pub fn is_saved_theme_dark() -> bool {
    read_pref_readonly("theme_mode")
        .map(|s| s == "dark")
        .unwrap_or(false)
}

/// 1 Hz poll of the shared prefs: returns the saved theme's dark flag when it
/// differs from `current_dark`, so long-lived viewer/analyzer subprocesses
/// follow main-app theme switches without needing a restart.
pub fn poll_saved_theme_change(
    current_dark: bool,
    last_poll: &mut std::time::Instant,
) -> Option<bool> {
    if last_poll.elapsed() < std::time::Duration::from_secs(1) {
        return None;
    }
    *last_poll = std::time::Instant::now();
    let saved = is_saved_theme_dark();
    (saved != current_dark).then_some(saved)
}

/// Build [`eframe::NativeOptions`] tuned for a low-baseline-RAM viewer
/// subprocess. See the module-level docs for the rationale of each knob.
pub fn build_viewer_native_options(viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        persist_window: false,
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    }
}
