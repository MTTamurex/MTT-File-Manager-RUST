//! View setup: computer view, recycle bin view, drive list
//!
//! This module handles setting up special views like "This PC" and "Recycle Bin".

use std::collections::HashSet;
use std::time::Instant;

use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows as windows_infra;

mod computer;
mod drive_scan_wake;
mod recycle_bin;

impl ImageViewerApp {
    fn refresh_inactive_computer_view_if_visible(&mut self) {
        if !self.dual_panel_enabled
            || !self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| snapshot.is_computer_view)
        {
            return;
        }

        self.with_inactive_panel(|app| {
            if app.navigation_state.is_computer_view {
                app.setup_computer_view();
            }
        });
    }

    fn refresh_visible_computer_views_after_drive_list_change(&mut self) -> bool {
        let mut refreshed_any = false;

        if self.navigation_state.is_computer_view {
            self.setup_computer_view();
            self.sync_to_tab();
            refreshed_any = true;
        }

        let had_inactive_computer = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_computer_view);
        self.refresh_inactive_computer_view_if_visible();
        refreshed_any || had_inactive_computer
    }

    pub(crate) fn remove_drive_from_visible_lists(&mut self, drive_path: &str) -> bool {
        if !self.drive_state.hide_drive_optimistically(drive_path) {
            return false;
        }

        crate::ui::sidebar::invalidate_drive_type_cache();
        self.drive_state.invalidate_drive_info_refreshes();
        self.drive_state.last_drive_bitmask =
            crate::infrastructure::windows::get_logical_drives_bitmask();
        let now = Instant::now();
        self.drive_state.last_drive_bitmask_check = now;
        self.drive_state.last_drive_full_refresh = now;
        if !self.refresh_visible_computer_views_after_drive_list_change() {
            self.refresh_drive_info_async();
        }
        self.ui_ctx.request_repaint();
        true
    }

    /// Launches a background thread to scan drives. Non-blocking.
    pub fn reload_drive_list_async(&mut self) {
        self.spawn_drive_list_scan(None);
    }

    /// Periodic fallback scan: an unchanged drive list does not wake the UI.
    pub(crate) fn reload_drive_list_quietly(&mut self) {
        let baseline = drive_scan_wake::DriveScanBaseline::capture(&self.drive_state);
        self.spawn_drive_list_scan(Some(baseline));
    }

    fn spawn_drive_list_scan(&mut self, baseline: Option<drive_scan_wake::DriveScanBaseline>) {
        if self.drive_state.drive_scan_pending {
            return; // Already scanning
        }
        self.drive_state.drive_scan_pending = true;
        let tx = self.drive_state.drive_scan_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            let (disks, cloud_roots, unavailable_label_roots) =
                crate::infrastructure::windows::get_drives_and_cloud_roots();
            let scan_result = crate::app::drive_state::DriveScanResult {
                disks,
                cloud_roots,
                network_locations: windows_infra::network_locations::get_network_locations(),
                unavailable_label_roots,
            };
            let wake_ui = drive_scan_wake::scan_wakes_ui(baseline.as_ref(), &scan_result);
            if tx.send(scan_result).is_ok() && wake_ui {
                ctx.request_repaint();
            }
        });
    }

    /// Poll for completed background drive scans. Called once per frame.
    pub fn poll_drive_scan(&mut self) {
        // Poll one-shot ISO detection from startup background thread.
        if let Some(rx) = &self.file_operation_state.iso_detect_rx {
            match rx.try_recv() {
                Ok(detected) => {
                    self.file_operation_state.iso_detect_rx = None;
                    if !detected.is_empty() {
                        log::info!(
                            "[ISO-DETECT] Merging {} pre-mounted ISO(s) into state",
                            detected.len()
                        );
                        for (detected_drive, iso_path) in detected {
                            let Some(drive) =
                                self.drive_state.canonical_current_drive(&detected_drive)
                            else {
                                log::debug!(
                                    "[ISO-DETECT] Ignoring stale or invalid drive mapping: {}",
                                    detected_drive
                                );
                                continue;
                            };
                            self.file_operation_state
                                .mounted_iso_drives
                                .entry(drive)
                                .or_insert(iso_path);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.file_operation_state.iso_detect_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }

        // PERF: Poll for deferred full drive/cloud detection from startup.
        // This avoids blocking the main thread on labels, registry scans and
        // sleeping HDD checks during cold start.
        if let Some(rx) = &self.drive_state.cloud_root_rx {
            match rx.try_recv() {
                Ok(scan_result) => {
                    self.drive_state.cloud_root_rx = None;
                    log::info!(
                        "[STARTUP] deferred drive/cloud scan received: {} drives, {} roots",
                        scan_result.disks.len(),
                        scan_result.cloud_roots.len()
                    );
                    let _ = self.drive_state.drive_scan_tx.send(scan_result);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.drive_state.cloud_root_rx = None;
                }
            }
        }

        match self.drive_state.drive_scan_rx.try_recv() {
            Ok(mut scan_result) => {
                self.drive_state.drive_scan_pending = false;
                self.drive_state
                    .apply_optimistic_drive_filter(&mut scan_result);
                self.drive_state
                    .preserve_labels_from_unavailable_queries(&mut scan_result);
                let old_disks = std::mem::take(&mut self.drive_state.disks);
                let old_cloud_roots = std::mem::take(&mut self.drive_state.cloud_roots);
                let cloud_roots_changed = scan_result.cloud_roots != old_cloud_roots;
                let network_locations_changed =
                    self.apply_network_location_scan(scan_result.network_locations);
                let changed = scan_result.disks != old_disks
                    || cloud_roots_changed
                    || network_locations_changed;
                self.drive_state.disks = scan_result.disks;
                self.drive_state.cloud_roots = scan_result.cloud_roots;
                self.item_icon_loader
                    .set_cloud_root_icon_resources(&self.drive_state.cloud_roots);
                self.item_icon_loader
                    .refresh_google_drive_root_icons(&self.drive_state.cloud_roots);

                if cloud_roots_changed {
                    let mut sync_root_paths: Vec<&str> = Vec::new();
                    let mut google_drive_paths: Vec<&str> = Vec::new();
                    for root in &self.drive_state.cloud_roots {
                        if root.is_google_drive_shortcut() {
                            google_drive_paths.extend(root.sync_root_paths());
                        }
                        sync_root_paths.extend(root.sync_root_paths());
                    }

                    crate::infrastructure::onedrive::refresh_cloud_sync_roots_from_paths(
                        sync_root_paths,
                    );
                    crate::infrastructure::onedrive::refresh_google_drive_roots_from_paths(
                        google_drive_paths,
                    );
                }

                if changed {
                    self.drive_state
                        .drive_health_scheduler
                        .reset_preload(std::time::Instant::now());
                    // Invalidate cached drive types since drive list changed
                    crate::ui::sidebar::invalidate_drive_type_cache();
                    let drive_roots_changed =
                        old_disks.iter().any(|(old_path, _)| {
                            !crate::app::drive_state::drive_root_is_present(
                                &self.drive_state.disks,
                                old_path,
                            )
                        }) || self.drive_state.disks.iter().any(|(new_path, _)| {
                            !crate::app::drive_state::drive_root_is_present(&old_disks, new_path)
                        });

                    // Fast startup enumeration uses provisional labels. A label
                    // change is not a volume change and must not erase capacity
                    // values that are already available for the same root.
                    if drive_roots_changed {
                        for (old_path, _) in &old_disks {
                            if !crate::app::drive_state::drive_root_is_present(
                                &self.drive_state.disks,
                                old_path,
                            ) {
                                if let Some(letter) = old_path.chars().next() {
                                    crate::infrastructure::windows::invalidate_physical_drive_cache(
                                        letter,
                                    );
                                }
                                self.drive_state.remove_cached_drive_info(old_path);
                            }
                        }
                        self.drive_state.invalidate_drive_info_refreshes();
                    }

                    // Tag assignments remain persistent while a root is
                    // unavailable. Reload visible tag views so unmounted paths
                    // disappear and return automatically after remounting.
                    self.refresh_tag_views_after_drive_change();

                    // Detect removed drives: if user is browsing inside a removed drive,
                    // navigate them to "Este Computador" to avoid showing stale cached data.
                    let reclassified_drive_roots: HashSet<String> = self
                        .drive_state
                        .cloud_roots
                        .iter()
                        .filter_map(|root| root.source_path.as_deref())
                        .map(str::to_owned)
                        .collect();
                    let removed_drives = super::drive_removal::removed_drive_roots(
                        &old_disks,
                        &self.drive_state.disks,
                        &reclassified_drive_roots,
                    );

                    if !removed_drives.is_empty() {
                        self.redirect_tabs_from_removed_drives(&removed_drives);
                    }

                    // AUTO-FOCUS FOR RECENTLY MOUNTED ISO
                    if let Some(iso_path) = self.file_operation_state.pending_iso_mount.take() {
                        let mut target_drive = None;
                        for (new_path, _label) in &self.drive_state.disks {
                            if !old_disks.iter().any(|(old_path, _)| old_path == new_path)
                                && crate::infrastructure::onedrive::fast_path_exists(
                                    std::path::Path::new(new_path),
                                )
                            {
                                target_drive = Some(new_path.clone());
                                break;
                            }
                        }

                        if let Some(drive) = target_drive {
                            self.file_operation_state
                                .mounted_iso_drives
                                .insert(drive.clone(), iso_path);
                            self.navigate_to(&drive);
                        } else {
                            self.file_operation_state.pending_iso_mount = Some(iso_path);
                        }
                    }

                    if !self.refresh_visible_computer_views_after_drive_list_change() {
                        self.refresh_drive_info_async();
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                if self.drive_state.drive_scan_pending {
                    log::warn!(
                        "[DRIVE-REFRESH] drive_scan channel disconnected; clearing pending flag"
                    );
                }
                self.drive_state.drive_scan_pending = false;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}
