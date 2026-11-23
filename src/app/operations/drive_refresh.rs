use std::time::{Duration, Instant};

use crate::app::drive_state::{merge_drive_info_query, DriveInfoRefreshScope};
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::DriveInfo;

const DRIVE_REFRESH_INTERVAL_MS: u64 = 30000;
const DRIVE_BITMASK_CHECK_INTERVAL_MS: u64 = 30000;
const DRIVE_INFO_REFRESH_INTERVAL_MS: u64 = 30000;
const REMOTE_DRIVE_INFO_REFRESH_INTERVAL_MS: u64 = 60000;
const DRIVE_INFO_REPAINT_COALESCE: Duration = Duration::from_millis(250);
mod computer_items;
mod worker;
/// Configured virtual drives (Cryptomator and similar) route every volume query
/// through a decrypting filesystem down to the backing HDD. Re-querying them
/// too frequently keeps waking that drive while browsing, so refresh them coarsely
/// and keep the last known values in between.
const VIRTUAL_DRIVE_INFO_REFRESH_INTERVAL_MS: u64 = 30000;

fn virtual_drive_info_query_times(
) -> &'static parking_lot::Mutex<rustc_hash::FxHashMap<char, Instant>> {
    static LAST_QUERY: std::sync::OnceLock<
        parking_lot::Mutex<rustc_hash::FxHashMap<char, Instant>>,
    > = std::sync::OnceLock::new();
    LAST_QUERY.get_or_init(|| parking_lot::Mutex::new(rustc_hash::FxHashMap::default()))
}

fn virtual_drive_info_query_is_due(letter: char, now: Instant) -> bool {
    !matches!(
        virtual_drive_info_query_times().lock().get(&letter),
        Some(last)
            if now.saturating_duration_since(*last)
                < Duration::from_millis(VIRTUAL_DRIVE_INFO_REFRESH_INTERVAL_MS)
    )
}

fn record_virtual_drive_info_query(letter: char, now: Instant) {
    virtual_drive_info_query_times()
        .lock()
        .insert(letter.to_ascii_uppercase(), now);
}

fn configured_virtual_drive_query_is_due(has_cached_capacity: bool, interval_is_due: bool) -> bool {
    !has_cached_capacity || interval_is_due
}

fn should_query_drive_info(path: &str, has_cached_capacity: bool, now: Instant) -> bool {
    let Some(letter) =
        crate::infrastructure::windows::extract_drive_letter(std::path::Path::new(path))
    else {
        return true;
    };
    if crate::infrastructure::virtual_drive_config::get_drive_override(letter).is_none() {
        return true;
    }
    configured_virtual_drive_query_is_due(
        has_cached_capacity,
        virtual_drive_info_query_is_due(letter, now),
    )
}

fn record_successful_virtual_drive_info_query(path: &str, now: Instant) {
    let Some(letter) =
        crate::infrastructure::windows::extract_drive_letter(std::path::Path::new(path))
    else {
        return;
    };
    if crate::infrastructure::virtual_drive_config::get_drive_override(letter).is_some() {
        record_virtual_drive_info_query(letter, now);
    }
}

fn take_due_drive_bitmask_check(last_check: &mut Instant, now: Instant) -> bool {
    if now.saturating_duration_since(*last_check)
        < Duration::from_millis(DRIVE_BITMASK_CHECK_INTERVAL_MS)
    {
        return false;
    }

    *last_check = now;
    true
}

fn drive_full_refresh_is_due(last_refresh: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_refresh) >= Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS)
}

fn drive_scope_matches(
    scope: DriveInfoRefreshScope,
    drive_type: crate::infrastructure::windows::DriveType,
) -> bool {
    match scope {
        DriveInfoRefreshScope::Local => !matches!(
            drive_type,
            crate::infrastructure::windows::DriveType::Remote
                | crate::infrastructure::windows::DriveType::Unknown
        ),
        DriveInfoRefreshScope::Remote => matches!(
            drive_type,
            crate::infrastructure::windows::DriveType::Remote
                | crate::infrastructure::windows::DriveType::Unknown
        ),
    }
}

fn merge_changed_drive_info(
    existing: Option<&DriveInfo>,
    queried: DriveInfo,
    capacity_query_succeeded: bool,
) -> Option<DriveInfo> {
    let merged = merge_drive_info_query(existing, queried, capacity_query_succeeded);
    (existing != Some(&merged)).then_some(merged)
}

fn record_latest_result(results: &mut Vec<(String, DriveInfo)>, root: String, info: DriveInfo) {
    if let Some((_, previous)) = results.iter_mut().find(|(path, _)| *path == root) {
        *previous = info;
    } else {
        results.push((root, info));
    }
}

impl ImageViewerApp {
    pub(crate) fn apply_drive_info_to_current_computer_items(
        &mut self,
        results: &[(String, DriveInfo)],
    ) {
        if !self.navigation_state.is_computer_view {
            return;
        }

        computer_items::apply_drive_info(self.all_items_mut(), results);
        self.sort_items();
        self.sync_selected_file_from_all_items();
        if !self.in_inactive_panel_context {
            self.sync_to_tab();
        }
    }

    pub fn refresh_drive_info_async(&mut self) {
        self.request_drive_info_refresh(DriveInfoRefreshScope::Local);
        self.request_drive_info_refresh(DriveInfoRefreshScope::Remote);
    }

    fn request_drive_info_refresh(&mut self, scope: DriveInfoRefreshScope) {
        let Some(generation) = self.drive_state.drive_info_refresh.begin(scope) else {
            return;
        };
        let disks_snapshot = self
            .drive_state
            .disks
            .iter()
            .map(|(path, _)| (path.clone(), self.drive_state.cached_drive_info(path)))
            .collect();
        worker::spawn(worker::Refresh {
            scope,
            generation,
            disks: disks_snapshot,
            tx: self.drive_state.drive_info_tx.clone(),
            ctx: self.ui_ctx.clone(),
            hardware: self.drive_state.drive_info_refresh.hardware_schedule(),
        });
    }

    pub fn refresh_drives_if_needed(&mut self) {
        let now = Instant::now();
        if take_due_drive_bitmask_check(&mut self.drive_state.last_drive_bitmask_check, now) {
            let current_bitmask = crate::infrastructure::windows::get_logical_drives_bitmask();
            if current_bitmask != self.drive_state.last_drive_bitmask {
                log::debug!(
                    "[DRIVE-REFRESH] Bitmask changed: 0x{:08X} -> 0x{:08X}",
                    self.drive_state.last_drive_bitmask,
                    current_bitmask
                );
                self.drive_state.last_drive_bitmask = current_bitmask;
                self.drive_state.last_drive_full_refresh = now;
                self.reload_drive_list_async();
            } else if drive_full_refresh_is_due(self.drive_state.last_drive_full_refresh, now) {
                self.drive_state.last_drive_full_refresh = now;
                self.reload_drive_list_async();
            }
        }

        if self.navigation_state.is_computer_view || self.show_left_sidebar {
            if !self
                .drive_state
                .drive_info_refresh
                .is_pending(DriveInfoRefreshScope::Local)
                && !self.drive_state.drive_health_scheduler.is_active()
                && self
                    .drive_state
                    .drive_info_refresh
                    .elapsed(DriveInfoRefreshScope::Local)
                    >= Duration::from_millis(DRIVE_INFO_REFRESH_INTERVAL_MS)
            {
                self.request_drive_info_refresh(DriveInfoRefreshScope::Local);
            }
            if !self
                .drive_state
                .drive_info_refresh
                .is_pending(DriveInfoRefreshScope::Remote)
                && self
                    .drive_state
                    .drive_info_refresh
                    .elapsed(DriveInfoRefreshScope::Remote)
                    >= Duration::from_millis(REMOTE_DRIVE_INFO_REFRESH_INTERVAL_MS)
            {
                self.request_drive_info_refresh(DriveInfoRefreshScope::Remote);
            }
        }
    }

    pub fn poll_drive_info(&mut self) {
        let mut applied_results = Vec::new();
        let mut rerun_local = false;
        let mut rerun_remote = false;

        while let Ok(result) = self.drive_state.drive_info_rx.try_recv() {
            let accepted = self
                .drive_state
                .drive_info_refresh
                .accepts(result.scope, result.generation);
            if accepted {
                for entry in result.entries {
                    let Some(root) = self.drive_state.canonical_current_drive(&entry.path) else {
                        continue;
                    };
                    let existing = self.drive_state.cached_drive_info(&entry.path);
                    let Some(info) = merge_changed_drive_info(
                        existing.as_ref(),
                        entry.info,
                        entry.capacity_query_succeeded,
                    ) else {
                        continue;
                    };
                    self.drive_state.cache_drive_info(&root, info.clone());
                    record_latest_result(&mut applied_results, root, info);
                }
            }

            if result.complete {
                let rerun = self.drive_state.drive_info_refresh.finish(
                    result.scope,
                    result.generation,
                    Instant::now(),
                );
                match result.scope {
                    DriveInfoRefreshScope::Local => rerun_local |= rerun,
                    DriveInfoRefreshScope::Remote => rerun_remote |= rerun,
                }
            }
        }

        if !applied_results.is_empty() {
            self.apply_drive_info_to_current_computer_items(&applied_results);
            if self.dual_panel_enabled
                && self
                    .dual_panel_inactive_state
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.is_computer_view)
            {
                self.with_inactive_panel(|app| {
                    app.apply_drive_info_to_current_computer_items(&applied_results);
                });
            }
        }
        if rerun_local {
            self.request_drive_info_refresh(DriveInfoRefreshScope::Local);
        }
        if rerun_remote {
            self.request_drive_info_refresh(DriveInfoRefreshScope::Remote);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::windows::DriveType;

    #[test]
    fn unavailable_unknown_roots_cannot_block_local_refreshes() {
        assert!(!drive_scope_matches(
            DriveInfoRefreshScope::Local,
            DriveType::Unknown
        ));
        assert!(drive_scope_matches(
            DriveInfoRefreshScope::Remote,
            DriveType::Unknown
        ));
    }

    #[test]
    fn bitmask_check_is_not_repeated_each_frame_after_deadline() {
        let start = Instant::now();
        let mut last_check = start;
        let deadline = start + Duration::from_millis(DRIVE_BITMASK_CHECK_INTERVAL_MS);

        assert!(!take_due_drive_bitmask_check(
            &mut last_check,
            deadline - Duration::from_millis(1)
        ));
        assert!(take_due_drive_bitmask_check(&mut last_check, deadline));
        assert!(!take_due_drive_bitmask_check(&mut last_check, deadline));
        assert!(!take_due_drive_bitmask_check(
            &mut last_check,
            deadline + Duration::from_millis(1)
        ));
    }

    #[test]
    fn bitmask_checks_do_not_postpone_full_refresh_deadline() {
        let start = Instant::now();
        let mut last_check = start;

        for seconds in (30..=120).step_by(30) {
            assert!(take_due_drive_bitmask_check(
                &mut last_check,
                start + Duration::from_secs(seconds)
            ));
        }

        assert!(!drive_full_refresh_is_due(
            start,
            start + Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS - 1)
        ));
        assert!(drive_full_refresh_is_due(
            start,
            start + Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS)
        ));
    }

    #[test]
    fn virtual_drive_info_queries_are_throttled() {
        let start = Instant::now();
        record_virtual_drive_info_query('Q', start);
        assert!(!virtual_drive_info_query_is_due(
            'Q',
            start + Duration::from_millis(5000)
        ));
        assert!(virtual_drive_info_query_is_due(
            'Q',
            start + Duration::from_millis(VIRTUAL_DRIVE_INFO_REFRESH_INTERVAL_MS)
        ));
    }

    #[test]
    fn virtual_drive_without_cached_capacity_bypasses_throttle() {
        assert!(configured_virtual_drive_query_is_due(false, false));
        assert!(!configured_virtual_drive_query_is_due(true, false));
        assert!(configured_virtual_drive_query_is_due(true, true));
    }

    #[test]
    fn unchanged_capacity_is_compared_after_preserving_cached_metadata() {
        let existing = DriveInfo {
            file_system: "NTFS".to_string(),
            total_space: 1_000,
            free_space: 400,
            drive_type: DriveType::Fixed,
            model: Some("model".to_string()),
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
            health: None,
        };
        let capacity_only = DriveInfo {
            file_system: String::new(),
            total_space: 1_000,
            free_space: 400,
            drive_type: DriveType::Fixed,
            model: None,
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
            health: None,
        };
        let merged = merge_drive_info_query(Some(&existing), capacity_only, true);
        assert_eq!(merged, existing);
    }
}
