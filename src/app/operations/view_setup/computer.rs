//! Assemble This PC from cached drives and saved network locations, without remote I/O.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{DriveInfo, FileEntry, SyncStatus};
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use crate::infrastructure::windows as windows_infra;
use std::path::PathBuf;
use std::sync::Arc;

impl ImageViewerApp {
    /// Sets up the "This PC" view without affecting history.
    pub fn setup_computer_view(&mut self) {
        self.active_tag_filter = None;
        // Invalidate results from the previous folder using the shared generation space.
        self.bump_folder_load_generation();
        self.invalidate_active_items_rebuild();
        if self.should_preserve_inactive_dual_panel_thumbnail_pipeline() {
            self.prune_thumbnail_pipeline_for_dual_panel_navigation("computer-view");
        } else {
            self.release_thumbnail_pipeline_for_inactive_view("computer-view", true);
        }

        self.navigation_state.current_path = COMPUTER_VIEW_ID.to_string();
        self.navigation_state.is_computer_view = true;
        self.navigation_state.is_recycle_bin_view = false;
        self.navigation_state.path_input = COMPUTER_VIEW_ID.to_string();
        self.apply_folder_lock_if_present();

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
                    model: None,
                    serial_number: None,
                    firmware_revision: None,
                    bus_type: None,
                    health: None,
                });
            computer_items.push(FileEntry {
                path: PathBuf::from(path),
                name: label.clone(),
                is_dir: true,
                size: 0,
                modified: 0,
                created: None,
                folder_cover: None,
                drive_info: Some(drive_info),
                sync_status: SyncStatus::None,
                is_hidden: false,
                recycle_bin: None,
            });
        }

        // Retain drive skeletons while capacity and hardware queries run asynchronously.
        for item in &computer_items {
            if let Some(info) = &item.drive_info {
                self.drive_state
                    .cache_drive_info(&item.path.to_string_lossy(), info.clone());
            }
        }
        computer_items.extend(
            self.drive_state
                .network_locations
                .iter()
                .map(|location| location.to_file_entry()),
        );

        self.all_items = Arc::new(computer_items);
        self.items_revision = self.items_revision.wrapping_add(1);
        // Sorting also rebuilds the grouped projection and the computer-view indices.
        self.sort_items();
        self.reset_selection_and_search();
        self.total_items = self.items.len();
        self.is_loading_folder = false;
        self.folder_load_error = None;
        self.refresh_drive_info_async();
    }
}
