//! Shared lightweight runtime helpers for the standalone viewer subprocesses
//! (`--image-viewer`, `--pdf-viewer`, `--docx-viewer`, `--text-viewer`).
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
//!     automatically retry with WGPU/DX12 if OpenGL initialization fails;
//!   - disable optional GL-only buffers (`depth_buffer`, `stencil_buffer`,
//!     `multisampling`) which the viewers never use.

use crate::app::navigation_state::ThemeMode;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

mod renderer;
pub use renderer::run_viewer_native;

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

pub fn saved_theme_mode() -> ThemeMode {
    read_pref_readonly("theme_mode")
        .map(|s| ThemeMode::from_preference(&s))
        .unwrap_or(ThemeMode::Light)
}

/// Whether the saved theme currently uses dark native window decorations.
pub fn is_saved_theme_dark() -> bool {
    saved_theme_mode().resolve_dark()
}

/// Detect palette changes as well as Windows light/dark changes at 1 Hz.
pub fn poll_saved_theme_change(ctx: &egui::Context, last_poll: &mut Instant) -> Option<ThemeMode> {
    let now = Instant::now();
    if now.duration_since(*last_poll) < THEME_POLL_INTERVAL {
        return None;
    }
    *last_poll = now;
    let saved = saved_theme_mode();
    theme_change(ctx, saved)
}

fn theme_change(ctx: &egui::Context, saved: ThemeMode) -> Option<ThemeMode> {
    (saved != crate::ui::theme::current_mode(ctx)
        || saved.resolve_dark() != ctx.global_style().visuals.dark_mode)
        .then_some(saved)
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

/// Keep standalone viewer controls at the same scale as the main file manager.
fn poll_saved_interface_scale(ctx: &egui::Context) {
    let id = egui::Id::new("saved_interface_scale_poll");
    let now = Instant::now();
    let cached = ctx.data(|data| data.get_temp::<(Instant, f32)>(id));
    let (last_poll, scale) = match cached {
        Some((last_poll, scale)) if now.duration_since(last_poll) < THEME_POLL_INTERVAL => {
            (last_poll, scale)
        }
        _ => {
            let value = read_pref_readonly(crate::ui::interface_scale::PREFERENCE_KEY);
            let scale = crate::ui::interface_scale::from_preference(value.as_deref());
            ctx.data_mut(|data| data.insert_temp(id, (now, scale)));
            (now, scale)
        }
    };
    crate::ui::interface_scale::apply(ctx, scale);
    ctx.request_repaint_after(theme_poll_remaining(now, last_poll));
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
                let selected = read_pref_readonly(crate::ui::fonts::selection::PREFERENCE_KEY)
                    .unwrap_or_default();
                if !selected.is_empty() {
                    fonts = crate::ui::fonts::selection::LoadedFonts::load(fonts, selected.clone())
                        .into_definitions();
                }
                let scale =
                    read_pref_readonly(&crate::ui::fonts::sizing::preference_key(&selected));
                crate::ui::fonts::sizing::apply(
                    &mut fonts,
                    crate::ui::fonts::sizing::from_preference(scale.as_deref()),
                );
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
    poll_saved_interface_scale(ctx);
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
    fn viewers_detect_changes_between_every_pair_of_dark_palettes() {
        let ctx = egui::Context::default();
        for current in [ThemeMode::Dark, ThemeMode::DarkDeep, ThemeMode::GitHubDark] {
            crate::ui::theme::apply_viewer_visuals(&ctx, current);
            for saved in [ThemeMode::Dark, ThemeMode::DarkDeep, ThemeMode::GitHubDark] {
                assert_eq!(
                    theme_change(&ctx, saved),
                    (saved != current).then_some(saved)
                );
            }
        }
    }

    #[test]
    fn viewers_detect_system_appearance_changes_even_when_mode_is_unchanged() {
        let ctx = egui::Context::default();
        crate::ui::theme::apply_viewer_visuals(&ctx, ThemeMode::System);
        let dark = ctx.global_style().visuals.dark_mode;
        ctx.global_style_mut(|style| style.visuals.dark_mode = !dark);
        assert_eq!(
            theme_change(&ctx, ThemeMode::System),
            Some(ThemeMode::System)
        );
    }

    #[test]
    fn theme_poll_does_not_read_preferences_before_its_interval() {
        let ctx = egui::Context::default();
        let mut last_poll = Instant::now();
        let previous = last_poll;
        assert_eq!(poll_saved_theme_change(&ctx, &mut last_poll), None);
        assert_eq!(last_poll, previous);
    }

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
