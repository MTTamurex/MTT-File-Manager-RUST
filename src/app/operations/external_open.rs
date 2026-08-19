//! Consumption of "open in main app" requests sent by child processes
//! (the standalone disk analyzer). Requests arrive either instantly via
//! WM_COPYDATA (queued by the window subclass) or through a `pending_open_request`
//! row in `app_state.db` written by a `--open-path` helper invocation when
//! the main window could not be reached directly.

use crate::app::ImageViewerApp;
use crate::infrastructure::app_state_db::AppStateDb;
use eframe::egui;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Preference key holding a pending open request from a child process.
pub const PENDING_OPEN_REQUEST_KEY: &str = "pending_open_request";
/// Requests older than this are ignored (stale rows from crashed helpers).
const DB_REQUEST_MAX_AGE_SECS: u64 = 600;
/// Throttle for the DB fallback poll (WM_COPYDATA is checked every frame).
const DB_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Encode a request row as `<unix_secs>|<path>` so staleness is detectable.
pub fn encode_db_request(path: &str) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}|{path}")
}

/// Drain queued external open requests and navigate to the first one.
/// Runs every frame from the app logic loop; gated until startup completes.
pub fn process_external_open_requests(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.startup_tick < 2 {
        return;
    }

    let queued = crate::infrastructure::windows::window_subclass::take_pending_open_requests();
    let path = match queued.into_iter().next() {
        Some(path) => Some(path),
        None => take_db_request(&app.app_state_db),
    };
    let Some(path) = path else {
        return;
    };

    log::info!("[EXTERNAL-OPEN] navigating to requested folder: {path}");
    app.navigate_to(&path);
    if let Some(hwnd) = app.native_hwnd {
        crate::infrastructure::windows::restore_window_foreground(hwnd);
    }
    ctx.request_repaint();
}

/// Consume a pending request from the shared preferences DB (throttled).
fn take_db_request(db: &Arc<AppStateDb>) -> Option<String> {
    static LAST_DB_POLL: OnceLock<Mutex<Instant>> = OnceLock::new();
    let gate = LAST_DB_POLL.get_or_init(|| Mutex::new(Instant::now() - DB_POLL_INTERVAL));
    {
        let Ok(mut last) = gate.lock() else {
            return None;
        };
        if last.elapsed() < DB_POLL_INTERVAL {
            return None;
        }
        *last = Instant::now();
    }

    let raw = db.get_preference(PENDING_OPEN_REQUEST_KEY)?;
    // Consume immediately so a restart or second poll never replays it.
    let _ = db.set_preference(PENDING_OPEN_REQUEST_KEY, "");

    let (secs_str, path) = raw.split_once('|')?;
    let secs: u64 = secs_str.parse().ok()?;
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    if now < secs || now - secs > DB_REQUEST_MAX_AGE_SECS {
        return None;
    }
    Some(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_db_request_round_trips_through_parser_rules() {
        let encoded = encode_db_request(r"C:\Some Folder");
        let (secs_str, path) = encoded.split_once('|').unwrap();
        let secs: u64 = secs_str.parse().unwrap();
        assert_eq!(path, r"C:\Some Folder");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(secs <= now && now - secs < 5);
    }
}
