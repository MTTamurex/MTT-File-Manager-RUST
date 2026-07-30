use std::time::{Duration, Instant};

use crate::app::drive_state::{
    merge_drive_info_query, DriveInfoRefreshEntry, DriveInfoRefreshResult, DriveInfoRefreshScope,
};
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::DriveInfo;

const DRIVE_REFRESH_INTERVAL_MS: u64 = 30000;
const DRIVE_BITMASK_CHECK_INTERVAL_MS: u64 = 3000;
const DRIVE_INFO_REFRESH_INTERVAL_MS: u64 = 5000;
const REMOTE_DRIVE_INFO_REFRESH_INTERVAL_MS: u64 = 60000;

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

fn query_drive_info(
    path: String,
    drive_type: crate::infrastructure::windows::DriveType,
) -> DriveInfoRefreshEntry {
    let vol = crate::infrastructure::windows::get_volume_info(&path);
    let hw = crate::infrastructure::windows::query_hardware_fields(&path, drive_type);
    DriveInfoRefreshEntry {
        path,
        capacity_query_succeeded: vol.capacity_query_succeeded,
        info: DriveInfo {
            file_system: vol.file_system,
            total_space: vol.total_space,
            free_space: vol.free_space,
            drive_type,
            model: hw.model,
            serial_number: hw.serial_number,
            firmware_revision: hw.firmware_revision,
            bus_type: hw.bus_type,
        },
    }
}

fn spawn_drive_info_refresh(
    scope: DriveInfoRefreshScope,
    generation: u64,
    disks_snapshot: Vec<String>,
    tx: std::sync::mpsc::Sender<DriveInfoRefreshResult>,
    ctx: eframe::egui::Context,
) {
    std::thread::spawn(move || {
        let queried = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut local_entries = Vec::new();
            for path in disks_snapshot {
                let drive_type = crate::infrastructure::windows::detect_drive_type(&path);
                if !drive_scope_matches(scope, drive_type) {
                    continue;
                }

                let entry = query_drive_info(path, drive_type);
                if scope == DriveInfoRefreshScope::Remote {
                    if tx
                        .send(DriveInfoRefreshResult {
                            scope,
                            generation,
                            entries: vec![entry],
                            complete: false,
                        })
                        .is_err()
                    {
                        return Vec::new();
                    }
                    ctx.request_repaint();
                } else {
                    local_entries.push(entry);
                }
            }
            local_entries
        }));

        let entries = match queried {
            Ok(entries) => entries,
            Err(_) => {
                log::error!("[DRIVE-REFRESH] Volume info worker panicked");
                Vec::new()
            }
        };
        let _ = tx.send(DriveInfoRefreshResult {
            scope,
            generation,
            entries,
            complete: true,
        });
        ctx.request_repaint();
    });
}

impl ImageViewerApp {
    fn apply_drive_info_to_current_computer_items(&mut self, results: &[(String, DriveInfo)]) {
        if !self.navigation_state.is_computer_view {
            return;
        }

        for item in self.all_items_mut().iter_mut() {
            let item_path = item.path.to_string_lossy();
            if let Some((_, info)) = results.iter().find(|(p, _)| p == item_path.as_ref()) {
                item.drive_info = Some(info.clone());
            }
        }
        self.sort_items();
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
            .map(|(path, _)| path.clone())
            .collect();
        spawn_drive_info_refresh(
            scope,
            generation,
            disks_snapshot,
            self.drive_state.drive_info_tx.clone(),
            self.ui_ctx.clone(),
        );
    }

    pub fn refresh_drives_if_needed(&mut self) {
        let elapsed = self.drive_state.last_drive_refresh.elapsed();

        if elapsed >= Duration::from_millis(DRIVE_BITMASK_CHECK_INTERVAL_MS) {
            let current_bitmask = crate::infrastructure::windows::get_logical_drives_bitmask();
            if current_bitmask != self.drive_state.last_drive_bitmask {
                log::debug!(
                    "[DRIVE-REFRESH] Bitmask changed: 0x{:08X} -> 0x{:08X}",
                    self.drive_state.last_drive_bitmask,
                    current_bitmask
                );
                self.drive_state.last_drive_bitmask = current_bitmask;
                self.drive_state.last_drive_refresh = Instant::now();
                self.reload_drive_list_async();
            } else if elapsed >= Duration::from_millis(DRIVE_REFRESH_INTERVAL_MS) {
                self.drive_state.last_drive_refresh = Instant::now();
                self.reload_drive_list_async();
            }
        }

        if self.navigation_state.is_computer_view || self.show_left_sidebar {
            if !self
                .drive_state
                .drive_info_refresh
                .is_pending(DriveInfoRefreshScope::Local)
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
                    if self
                        .drive_state
                        .canonical_current_drive(&entry.path)
                        .is_none()
                    {
                        continue;
                    }
                    let existing = self.drive_state.cached_drive_info(&entry.path);
                    let info = merge_drive_info_query(
                        existing.as_ref(),
                        entry.info,
                        entry.capacity_query_succeeded,
                    );
                    self.drive_state.cache_drive_info(&entry.path, info.clone());
                    applied_results.push((entry.path, info));
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
}
