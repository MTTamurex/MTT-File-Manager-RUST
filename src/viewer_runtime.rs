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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const THEME_POLL_INTERVAL: Duration = Duration::from_secs(1);

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

/// Returns `true` if the saved theme renders dark right now: `"dark"` always
/// does, `"system"` follows the Windows app mode, and anything else (including
/// no stored preference or light) is `false`.
pub fn is_saved_theme_dark() -> bool {
    read_pref_readonly("theme_mode")
        .map(|s| match s.as_str() {
            "dark" => true,
            "system" => crate::infrastructure::windows::windows_app_is_dark(),
            _ => false,
        })
        .unwrap_or(false)
}

/// 1 Hz poll of the shared prefs: returns the saved theme's dark flag when it
/// differs from `current_dark`, so long-lived viewer/analyzer subprocesses
/// follow main-app theme switches without needing a restart.
pub fn poll_saved_theme_change(current_dark: bool, last_poll: &mut Instant) -> Option<bool> {
    let now = Instant::now();
    if now.duration_since(*last_poll) < THEME_POLL_INTERVAL {
        return None;
    }
    *last_poll = now;
    let saved = is_saved_theme_dark();
    (saved != current_dark).then_some(saved)
}

/// Schedules the frame that makes the next saved-theme poll eligible. This
/// keeps an idle viewer in sync when a Windows theme notification arrives
/// during the poll interval.
pub fn schedule_saved_theme_poll(ctx: &egui::Context, last_poll: Instant) {
    ctx.request_repaint_after(theme_poll_remaining(Instant::now(), last_poll));
}

fn theme_poll_remaining(now: Instant, last_poll: Instant) -> Duration {
    THEME_POLL_INTERVAL.saturating_sub(now.duration_since(last_poll))
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

/// Tracks the background font-loading lifecycle so the UI thread can poll for
/// the result without ever blocking, and without re-polling after it is applied.
enum ViewerFontState {
    Loading,
    Ready(egui::FontDefinitions),
    Applied,
}

/// Process-wide, lazily-started font loader shared by every viewer window in
/// this process. Kept out of the UI thread so reading the ~tens of MB of system
/// font files never blocks the first frame.
fn viewer_font_cell() -> &'static Arc<Mutex<ViewerFontState>> {
    static CELL: OnceLock<Arc<Mutex<ViewerFontState>>> = OnceLock::new();
    CELL.get_or_init(|| {
        let cell = Arc::new(Mutex::new(ViewerFontState::Loading));
        let thread_cell = Arc::clone(&cell);
        let spawn = std::thread::Builder::new()
            .name("viewer-font-loader".to_owned())
            .spawn(move || {
                let mut fonts = egui::FontDefinitions::default();
                crate::ui::fonts::install_system_fonts(&mut fonts);
                if let Ok(mut state) = thread_cell.lock() {
                    *state = ViewerFontState::Ready(fonts);
                }
            });

        // A spawn failure is extremely unlikely, but transition to `Applied`
        // immediately so the polling loop below can't spin forever.
        if spawn.is_err() {
            if let Ok(mut state) = cell.lock() {
                *state = ViewerFontState::Applied;
            }
        }

        cell
    })
}

/// Polls the background font loader and applies the definitions once they are
/// ready, returning `true` when applied.
///
/// While loading, it schedules a short repaint so the caller keeps polling
/// without blocking the UI thread. Callers should invoke this every frame.
pub fn poll_viewer_fonts(ctx: &egui::Context) -> bool {
    let cell = viewer_font_cell();
    let mut state = cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    if matches!(*state, ViewerFontState::Ready(_)) {
        let previous = std::mem::replace(&mut *state, ViewerFontState::Applied);
        drop(state);
        if let ViewerFontState::Ready(fonts) = previous {
            ctx.set_fonts(fonts);
            ctx.request_repaint();
        }
        return true;
    }

    let loading = matches!(*state, ViewerFontState::Loading);
    drop(state);
    if loading {
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_poll_remaining_waits_only_until_the_next_interval() {
        let now = Instant::now();
        assert_eq!(theme_poll_remaining(now, now), THEME_POLL_INTERVAL);
        assert_eq!(
            theme_poll_remaining(now, now.checked_sub(Duration::from_secs(2)).unwrap()),
            Duration::ZERO
        );
    }
}
