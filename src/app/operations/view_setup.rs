//! View setup: computer view, recycle bin view, drive list
//!
//! This module handles setting up special views like "This PC" and "Recycle Bin".

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{DriveInfo, FileEntry};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use crate::infrastructure::windows as windows_infra;

// PERFORMANCE: Increased from 2s to 30s to avoid periodic HDD access.
// Device insertion/removal is detected instantly via RegisterDeviceNotificationW
// (device_event_receiver in message_handler.rs). This timer is only a safety fallback.
const DRIVE_REFRESH_INTERVAL_MS: u64 = 30000;

// Fast bitmask check interval for virtual/mapped drives that don't fire WM_DEVICECHANGE.
// GetLogicalDrives() is instantaneous (kernel cache, no disk I/O).
const DRIVE_BITMASK_CHECK_INTERVAL_MS: u64 = 3000;

// Interval for re-reading volume info (free/total space) while the user
// stays in the "This PC" view. 5s is conservative and avoids hammering
// network drives while still feeling responsive for local changes.
const DRIVE_INFO_REFRESH_INTERVAL_MS: u64 = 5000;

fn normalize_drive_root_for_compare(path: &str) -> String {
    path.to_lowercase()
        .trim_end_matches(['\\', '/'])
        .to_string()
}

impl ImageViewerApp {
    pub fn setup_recycle_bin_view(&mut self) {
        self.navigation_state.current_path = RECYCLE_BIN_VIEW_ID.to_string();
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = true;
        self.navigation_state.path_input = RECYCLE_BIN_VIEW_ID.to_string();

        // Restore unlocked defaults for settings that folder_lock may have overridden.
        // "Lixeira" cannot be locked, so always clear the locked state.
        self.sort_descending = self.sort_descending_normal;
        self.folders_position = self.folders_position_normal;
        self.view_mode = self.view_mode_normal;
        self.current_folder_locked = false;

        self.is_loading_folder = true;
        self.folder_load_error = None;
        self.items = Arc::new(Vec::new());
        self.all_items_mut().clear();
        self.total_items = 0;
        self.reset_selection_and_search();

        // Use the same globally-unique generation space as folder loads so
        // delayed dual-panel batches cannot be routed into this special view.
        self.bump_folder_load_generation();
        self.release_thumbnail_pipeline_for_inactive_view("recycle-bin", true);

        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();

        // Load Recycle Bin items in a separate thread (ASYNC) with batching
        std::thread::spawn(move || {
            use crate::infrastructure::windows::recycle_bin::enumerate_recycle_bin;

            // Enumerate Recycle Bin items via COM
            match enumerate_recycle_bin() {
                Ok(recycle_items) => {
                    const BATCH_SIZE: usize = 100;
                    let mut batch = Vec::with_capacity(BATCH_SIZE);

                    for item in recycle_items {
                        // Check if generation is still valid (fast cancellation)
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }

                        // Create a "virtual" path based on extension to load the correct icon
                        // The real path no longer exists, but the icon is based on the extension
                        // The real path ($R) is needed to read the deletion date ($I creation time)
                        // If physical_path is empty (read failure), fall back to the old dummy logic.
                        let file_path = if !item.physical_path.as_os_str().is_empty() {
                            item.physical_path.clone()
                        } else if item.is_directory {
                            PathBuf::from("C:\\folder")
                        } else if !item.extension.is_empty() {
                            PathBuf::from(format!("dummy{}", item.extension))
                        } else {
                            item.original_path.clone()
                        };

                        let entry = FileEntry {
                            path: file_path, // Physical path ($R) to allow get_deletion_date
                            name: item.name,
                            is_dir: item.is_directory,
                            size: item.size,
                            // Store deletion timestamp for stable numeric sort in Recycle Bin view.
                            modified: item.date_deleted_unix,
                            created: None,
                            folder_cover: None,
                            drive_info: None,
                            sync_status: crate::domain::file_entry::SyncStatus::None,
                            is_hidden: false,
                            recycle_bin: Some(Box::new(
                                crate::domain::file_entry::RecycleBinMeta {
                                    deletion_date: item.date_deleted,
                                    original_path: item.original_path,
                                },
                            )),
                        };
                        batch.push(entry);

                        // Send batch when full
                        if batch.len() >= BATCH_SIZE {
                            if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                return;
                            }
                            let _ = file_entry_sender.send((my_gen, std::mem::take(&mut batch)));
                            ctx.request_repaint();
                            batch = Vec::with_capacity(BATCH_SIZE);
                        }
                    }

                    // Send remaining items
                    if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, batch));
                        ctx.request_repaint();
                    }

                    // End-of-loading signal
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                    }
                }
                Err(e) => {
                    log::error!("[RECYCLE BIN] Error enumerating: {:?}", e);
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Sets up the "This PC" view without affecting history
    pub fn setup_computer_view(&mut self) {
        // CRITICAL: Advance generation to invalidate any pending items_rebuild results
        // from the previous folder load. Without this, stale rebuild results (matching
        // the old generation) arrive on the next frame and overwrite our computer view items.
        // Use the same globally-unique generation space as folder loads so
        // delayed dual-panel batches cannot be routed into this special view.
        self.bump_folder_load_generation();
        self.invalidate_active_items_rebuild();
        self.release_thumbnail_pipeline_for_inactive_view("computer-view", true);

        // Set computer view
        self.navigation_state.current_path = COMPUTER_VIEW_ID.to_string();
        self.navigation_state.is_computer_view = true;
        self.navigation_state.is_recycle_bin_view = false;
        self.navigation_state.path_input = COMPUTER_VIEW_ID.to_string();

        // Load Computer View sort mode
        self.sort_mode = self.sort_mode_computer;

        // Restore unlocked defaults for settings that folder_lock may have overridden.
        // "Este Computador" cannot be locked, so always clear the locked state.
        self.sort_descending = self.sort_descending_normal;
        self.folders_position = self.folders_position_normal;
        self.view_mode = self.view_mode_normal;
        self.current_folder_locked = false;

        // Populate items with drives using FAST-ONLY calls (no I/O blocking)
        // detect_drive_type uses GetDriveTypeW which is cached and < 1ms
        // get_volume_info is deferred to background thread
        let mut computer_items = Vec::new();
        for (path, label) in &self.drive_state.disks {
            let drive_type = windows_infra::detect_drive_type(path);
            let drive_info = self
                .drive_state
                .cached_drive_info(path)
                .unwrap_or(DriveInfo {
                    file_system: String::new(),
                    total_space: 0,
                    free_space: 0,
                    drive_type,
                });
            let entry = FileEntry {
                path: PathBuf::from(path),
                name: label.clone(),
                is_dir: true,
                size: 0,
                modified: 0,
                created: None,
                folder_cover: None,
                drive_info: Some(drive_info),
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: None,
            };
            computer_items.push(entry);
        }

        // Populate drive_info_cache with skeleton entries so the details panel
        // can show drive_type immediately, even before background volume info arrives.
        for item in &computer_items {
            if let Some(info) = &item.drive_info {
                let path_str = item.path.to_string_lossy().to_string();
                self.drive_state.cache_drive_info(&path_str, info.clone());
            }
        }

        self.all_items = Arc::new(computer_items);
        self.share_visible_items_from_all_items();

        // PRE-COMPUTE SECTION INDICES (O(n) once, not per frame)
        self.navigation_state.computer_view_local_indices.clear();
        self.navigation_state.computer_view_network_indices.clear();

        for (i, item) in self.items.iter().enumerate() {
            let is_remote = item.drive_info.as_ref().is_some_and(|di| {
                di.drive_type == crate::infrastructure::windows::DriveType::Remote
            });
            if is_remote {
                self.navigation_state.computer_view_network_indices.push(i);
            } else {
                self.navigation_state.computer_view_local_indices.push(i);
            }
        }

        self.reset_selection_and_search();
        self.total_items = self.drive_state.disks.len();
        self.is_loading_folder = false;
        self.folder_load_error = None;

        // Launch background thread for volume info (total/free space, file_system)
        self.drive_state.drive_info_refresh_pending = true;
        self.drive_state.last_drive_info_refresh = Instant::now();
        let disks_snapshot: Vec<String> = self
            .drive_state
            .disks
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        let tx = self.drive_state.drive_info_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            use crate::infrastructure::windows::get_volume_info;
            let mut results = Vec::new();
            for path in &disks_snapshot {
                let vol = get_volume_info(path);
                let drive_type = crate::infrastructure::windows::detect_drive_type(path);
                results.push((
                    path.clone(),
                    DriveInfo {
                        file_system: vol.file_system,
                        total_space: vol.total_space,
                        free_space: vol.free_space,
                        drive_type,
                    },
                ));
            }
            let incomplete_queries = results
                .iter()
                .filter(|(_, info)| info.total_space == 0 && info.free_space == 0)
                .count();
            if incomplete_queries > 0 {
                log::warn!(
                    "[DRIVE-REFRESH] Volume info refresh completed with incomplete results drives={} incomplete={}",
                    disks_snapshot.len(),
                    incomplete_queries
                );
            }
            let _ = tx.send(results);
            ctx.request_repaint();
        });
    }

    /// Launches a background thread to scan drives. Non-blocking.
    pub fn reload_drive_list_async(&mut self) {
        if self.drive_state.drive_scan_pending {
            return; // Already scanning
        }
        self.drive_state.drive_scan_pending = true;
        let tx = self.drive_state.drive_scan_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            let (disks, cloud_roots) = crate::infrastructure::windows::get_drives_and_cloud_roots();
            let scan_result = crate::app::drive_state::DriveScanResult { disks, cloud_roots };
            let _ = tx.send(scan_result);
            ctx.request_repaint();
        });
    }

    /// Launches a background thread to refresh volume info for all current drives.
    /// Non-blocking; guarded by drive_info_refresh_pending to avoid duplicates.
    pub fn refresh_drive_info_async(&mut self) {
        if self.drive_state.drive_info_refresh_pending {
            return;
        }
        self.drive_state.drive_info_refresh_pending = true;
        self.drive_state.last_drive_info_refresh = Instant::now();

        let disks_snapshot: Vec<String> = self
            .drive_state
            .disks
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        let tx = self.drive_state.drive_info_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            use crate::infrastructure::windows::get_volume_info;
            let mut results = Vec::new();
            for path in &disks_snapshot {
                let vol = get_volume_info(path);
                let drive_type = crate::infrastructure::windows::detect_drive_type(path);
                results.push((
                    path.clone(),
                    DriveInfo {
                        file_system: vol.file_system,
                        total_space: vol.total_space,
                        free_space: vol.free_space,
                        drive_type,
                    },
                ));
            }
            let _ = tx.send(results);
            ctx.request_repaint();
        });
    }

    /// Poll for completed background drive scans. Called once per frame.
    pub fn poll_drive_scan(&mut self) {
        match self.drive_state.drive_scan_rx.try_recv() {
            Ok(scan_result) => {
                self.drive_state.drive_scan_pending = false;
                let old_disks = std::mem::take(&mut self.drive_state.disks);
                let old_cloud_roots = std::mem::take(&mut self.drive_state.cloud_roots);
                let cloud_roots_changed = scan_result.cloud_roots != old_cloud_roots;
                let changed = scan_result.disks != old_disks || cloud_roots_changed;
                self.drive_state.disks = scan_result.disks;
                self.drive_state.cloud_roots = scan_result.cloud_roots;
                self.item_icon_loader
                    .set_cloud_root_icon_resources(&self.drive_state.cloud_roots);

                if cloud_roots_changed {
                    crate::infrastructure::onedrive::refresh_cloud_sync_roots_from_paths(
                        self.drive_state
                            .cloud_roots
                            .iter()
                            .filter(|root| root.is_windows_cloud_files())
                            .map(|root| root.path.as_str()),
                    );
                }

                if changed {
                    // Invalidate cached drive types since drive list changed
                    crate::ui::sidebar::invalidate_drive_type_cache();
                    self.drive_state.clear_cached_drive_info();
                    self.drive_state.drive_info_refresh_pending = false;

                    // Detect removed drives: if user is browsing inside a removed drive,
                    // navigate them to "Este Computador" to avoid showing stale cached data.
                    let reclassified_drive_roots: HashSet<String> = self
                        .drive_state
                        .cloud_roots
                        .iter()
                        .filter_map(|root| root.source_path.as_deref())
                        .map(normalize_drive_root_for_compare)
                        .collect();
                    let removed_drives: Vec<String> = old_disks
                        .iter()
                        .filter(|(old_path, _)| {
                            let old_key = normalize_drive_root_for_compare(old_path);
                            !reclassified_drive_roots.contains(&old_key)
                                && !self
                                    .drive_state
                                    .disks
                                    .iter()
                                    .any(|(new_path, _)| new_path == old_path)
                        })
                        .map(|(path, _)| path.clone())
                        .collect();

                    if !removed_drives.is_empty() {
                        for drive in &removed_drives {
                            self.file_operation_state.mounted_iso_drives.remove(drive);
                        }

                        log::info!("[DRIVE-REFRESH] Drives removed: {:?}", removed_drives);

                        // Check if user is currently browsing inside a removed drive
                        let current = self.navigation_state.current_path.clone();
                        let on_removed_drive = !self.navigation_state.is_computer_view
                            && !self.navigation_state.is_recycle_bin_view
                            && removed_drives.iter().any(|d| current.starts_with(d));

                        if on_removed_drive {
                            log::warn!(
                                "[DRIVE-REFRESH] Current path '{}' is on a removed drive, redirecting to Este Computador",
                                current
                            );
                            self.directory_cache.clear();
                            self.navigate_to_computer();
                            return;
                        }
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

                    if self.navigation_state.is_computer_view {
                        self.setup_computer_view();
                    } else {
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

    pub fn refresh_drives_if_needed(&mut self) {
        let elapsed = self.drive_state.last_drive_refresh.elapsed();

        // Fast check: compare drive bitmask every 3s (no disk I/O, reads kernel cache).
        // This catches virtual/mapped drives (Cryptomator, VeraCrypt, subst, net use)
        // that don't fire WM_DEVICECHANGE when unmounted.
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
                // Full fallback refresh every 30s (safety net)
                self.drive_state.last_drive_refresh = Instant::now();
                self.reload_drive_list_async();
            }
        }

        // While in computer view, periodically refresh volume info (free/total space)
        // so drive slots update without requiring the user to leave and re-enter.
        if self.navigation_state.is_computer_view
            && !self.drive_state.drive_info_refresh_pending
            && self.drive_state.last_drive_info_refresh.elapsed()
                >= Duration::from_millis(DRIVE_INFO_REFRESH_INTERVAL_MS)
        {
            self.refresh_drive_info_async();
        }
    }

    /// Poll for completed background volume info scans. Called once per frame.
    /// Updates drive_info (total_space, free_space, file_system) in existing items.
    pub fn poll_drive_info(&mut self) {
        let mut any_received = false;
        while let Ok(results) = self.drive_state.drive_info_rx.try_recv() {
            any_received = true;
            // Always persist drive info in the dedicated cache so it survives
            // navigation away from computer view (used by details panel).
            for (path, info) in &results {
                self.drive_state.cache_drive_info(path, info.clone());
            }

            if self.navigation_state.is_computer_view {
                // Update all_items with the received drive info
                for item in self.all_items_mut().iter_mut() {
                    let item_path = item.path.to_string_lossy();
                    if let Some((_, info)) = results.iter().find(|(p, _)| p == item_path.as_ref()) {
                        item.drive_info = Some(info.clone());
                    }
                }
                self.share_visible_items_from_all_items();
            }
        }
        if any_received {
            self.drive_state.drive_info_refresh_pending = false;
        }
    }
}
