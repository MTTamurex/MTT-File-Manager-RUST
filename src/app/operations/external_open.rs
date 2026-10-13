//! Consumption of item-reveal requests from child processes and new-tab folder
//! activations from Windows Explorer. Requests arrive via WM_COPYDATA or, for
//! the disk analyzer fallback, a `pending_open_request` row in `app_state.db`.

use crate::app::dual_panel::ActivePanel;
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
const DIRECTORY_TARGET_PREFIX: &str = "D|";
const FILE_TARGET_PREFIX: &str = "F|";
const OPEN_FOLDER_NEW_TAB_PREFIX: &str = "N|";

/// Encode the target kind without relying on filesystem metadata in the
/// receiving process. Unprefixed legacy requests remain directory requests.
pub fn encode_target_request(path: &str, is_dir: bool) -> String {
    format!(
        "{}{path}",
        if is_dir {
            DIRECTORY_TARGET_PREFIX
        } else {
            FILE_TARGET_PREFIX
        }
    )
}

/// Encode a Windows Explorer folder activation as an explicit new-tab request.
pub fn encode_open_folder_request(path: &str) -> String {
    format!("{OPEN_FOLDER_NEW_TAB_PREFIX}{path}")
}

fn decode_open_folder_request(request: &str) -> Option<&str> {
    request
        .strip_prefix(OPEN_FOLDER_NEW_TAB_PREFIX)
        .filter(|path| !path.is_empty())
}

fn decode_target_request(request: &str) -> (&str, bool) {
    if let Some(path) = request.strip_prefix(FILE_TARGET_PREFIX) {
        (path, false)
    } else if let Some(path) = request.strip_prefix(DIRECTORY_TARGET_PREFIX) {
        (path, true)
    } else {
        (request, true)
    }
}

/// Encode a request row as `<unix_secs>|<path>` so staleness is detectable.
pub fn encode_db_request(path: &str) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}|{path}")
}

/// Drain one queued external open request.
/// Runs every frame from the app logic loop; gated until startup completes.
pub fn process_external_open_requests(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.startup_tick < 2 {
        return;
    }

    let queued = crate::infrastructure::windows::window_subclass::take_pending_open_request();
    let path = match queued {
        Some(path) => Some(path),
        None => take_db_request(&app.app_state_db),
    };
    let Some(request) = path else {
        return;
    };
    if let Some(path) = decode_open_folder_request(&request) {
        match crate::app::startup_folder::validate_requested_folder(std::path::PathBuf::from(path))
        {
            Ok(path) => {
                app.open_integrated_folder_in_new_context(&path);
                if let Some(hwnd) = app.native_hwnd {
                    crate::infrastructure::windows::restore_window_foreground(hwnd);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.request_repaint();
            }
            Err(()) => log::warn!("[EXTERNAL-OPEN] Ignored an invalid Explorer folder request"),
        }
        return;
    }
    let (path, is_dir) = decode_target_request(&request);
    if path.is_empty() {
        return;
    }

    log::info!("[EXTERNAL-OPEN] revealing requested item: {path}");
    crate::ui::global_search_overlay::actions::activate_search_result(app, path, is_dir);
    if let Some(hwnd) = app.native_hwnd {
        crate::infrastructure::windows::restore_window_foreground(hwnd);
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.request_repaint();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegratedFolderDestination {
    MainTab,
    Panel(ActivePanel),
}

fn integrated_folder_destination(
    dual_panel_enabled: bool,
    focused_panel: ActivePanel,
) -> IntegratedFolderDestination {
    if dual_panel_enabled {
        IntegratedFolderDestination::Panel(focused_panel)
    } else {
        IntegratedFolderDestination::MainTab
    }
}

impl ImageViewerApp {
    pub(crate) fn open_integrated_folder_in_new_context(&mut self, path: &str) -> bool {
        if let IntegratedFolderDestination::Panel(panel) =
            integrated_folder_destination(self.dual_panel_enabled, self.dual_panel_active)
        {
            return self.dual_panel_add_tab_at_end(panel, path);
        }

        if !self.tab_manager.can_add_tab() {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return false;
        }

        let view_mode = self.view_mode;
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let folders_position = self.folders_position;
        self.sync_to_tab();
        if !self.tab_manager.new_tab_at(path) {
            return false;
        }
        {
            let active = self.tab_manager.active_mut();
            active.view_mode = view_mode;
            active.sort_mode = sort_mode;
            active.sort_descending = sort_descending;
            active.folders_position = folders_position;
        }
        self.sync_from_tab();
        self.sync_to_tab();
        self.update_video_visibility();
        self.save_preferences();
        true
    }
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

    #[test]
    fn target_request_preserves_path_and_item_kind() {
        let file = encode_target_request(r"C:\Some Folder\report.pdf", false);
        let directory = encode_target_request(r"C:\Some Folder", true);

        assert_eq!(
            decode_target_request(&file),
            (r"C:\Some Folder\report.pdf", false)
        );
        assert_eq!(decode_target_request(&directory), (r"C:\Some Folder", true));
    }

    #[test]
    fn legacy_target_request_remains_a_directory_request() {
        assert_eq!(
            decode_target_request(r"C:\Legacy Folder"),
            (r"C:\Legacy Folder", true)
        );
    }

    #[test]
    fn windows_folder_activation_uses_a_distinct_new_tab_request() {
        let request = encode_open_folder_request(r"C:\Users\user\Documents\Project");
        assert_eq!(
            decode_open_folder_request(&request),
            Some(r"C:\Users\user\Documents\Project")
        );
        assert_eq!(decode_open_folder_request("N|"), None);
    }

    #[test]
    fn integrated_folder_targets_outer_tabs_in_mono_and_last_focused_panel_in_dual() {
        assert_eq!(
            integrated_folder_destination(false, ActivePanel::Right),
            IntegratedFolderDestination::MainTab
        );
        assert_eq!(
            integrated_folder_destination(true, ActivePanel::Right),
            IntegratedFolderDestination::Panel(ActivePanel::Right)
        );
    }
}
