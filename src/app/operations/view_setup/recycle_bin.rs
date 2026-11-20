use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, RecycleBinMeta, SyncStatus};
use crate::domain::special_paths::RECYCLE_BIN_VIEW_ID;
use crate::infrastructure::windows::recycle_bin::{
    enumerate_recycle_bin_streaming_with, RecycleBinItem,
};

const RECYCLE_BIN_REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);
const RECYCLE_BIN_BATCH_SIZE: usize = 32;

impl ImageViewerApp {
    pub fn setup_recycle_bin_view(&mut self) {
        self.active_tag_filter = None;
        self.navigation_state.current_path = RECYCLE_BIN_VIEW_ID.to_string();
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = true;
        self.navigation_state.path_input = RECYCLE_BIN_VIEW_ID.to_string();

        // Restore unlocked defaults for settings that folder_lock may have overridden.
        self.sort_descending = self.sort_descending_normal;
        self.folders_position = self.folders_position_normal;
        self.view_mode = self.view_mode_normal;
        self.group_mode = self.group_mode_normal;
        self.group_descending = self.group_descending_normal;
        self.current_folder_locked = false;

        self.is_loading_folder = true;
        self.loading_started_at = Instant::now();
        self.pending_all_items_clear = false;
        self.hold_visible_items_until_load_complete = false;
        self.invalidate_active_items_rebuild();
        self.folder_load_error = None;
        self.items = Arc::new(Vec::new());
        self.group_projection = Arc::new(Default::default());
        self.all_items_mut().clear();
        self.total_items = 0;
        self.reset_selection_and_search();

        // Use the shared generation space so delayed batches cannot target this view.
        self.bump_folder_load_generation();
        if self.should_preserve_inactive_dual_panel_thumbnail_pipeline() {
            self.prune_thumbnail_pipeline_for_dual_panel_navigation("recycle-bin");
        } else {
            self.release_thumbnail_pipeline_for_inactive_view("recycle-bin", true);
        }

        let my_gen = self.generation;
        let generation = self.folder_load_generation.clone();
        let sender = self.file_entry_sender.clone();
        let failure_sender = self.folder_load_failure_sender.clone();
        let ctx = self.ui_ctx.clone();

        std::thread::spawn(move || {
            let result = enumerate_recycle_bin_streaming_with(
                |recycle_items| {
                    if generation.load(Ordering::Relaxed) != my_gen {
                        return;
                    }

                    let entries = recycle_items
                        .into_iter()
                        .map(recycle_bin_item_to_file_entry)
                        .collect();
                    let _ = sender.send((my_gen, entries));
                    ctx.request_repaint();
                },
                generation.clone(),
                my_gen,
                RECYCLE_BIN_BATCH_SIZE,
            );

            if generation.load(Ordering::Relaxed) == my_gen {
                if let Err(error) = result {
                    log::error!("[RECYCLE BIN] Error enumerating: {error:?}");
                    let _ = failure_sender.send((
                        my_gen,
                        crate::app::state::FolderLoadError::other(
                            PathBuf::from(RECYCLE_BIN_VIEW_ID),
                            error.to_string(),
                        ),
                    ));
                    ctx.request_repaint();
                    return;
                }
                let _ = sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }
        });
    }

    pub(crate) fn queue_recycle_bin_refresh(&mut self) {
        if self.has_visible_recycle_bin_view() {
            self.recycle_bin_refresh_pending_at = Some(Instant::now());
            self.ui_ctx
                .request_repaint_after(RECYCLE_BIN_REFRESH_DEBOUNCE);
        }
    }

    pub(crate) fn poll_recycle_bin_notifications(&mut self, ctx: &eframe::egui::Context) {
        if crate::infrastructure::windows::recycle_bin::take_change_notification() {
            self.queue_recycle_bin_refresh();
        }

        if !self.has_visible_recycle_bin_view() {
            self.recycle_bin_refresh_pending_at = None;
            return;
        }

        let Some(pending_at) = self.recycle_bin_refresh_pending_at else {
            return;
        };
        let elapsed = pending_at.elapsed();
        if elapsed < RECYCLE_BIN_REFRESH_DEBOUNCE {
            ctx.request_repaint_after(RECYCLE_BIN_REFRESH_DEBOUNCE - elapsed);
            return;
        }

        self.recycle_bin_refresh_pending_at = None;
        let refresh_active = self.navigation_state.is_recycle_bin_view;
        let refresh_inactive = self.dual_panel_enabled
            && self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| snapshot.is_recycle_bin_view);

        if refresh_active {
            self.setup_recycle_bin_view();
        }
        if refresh_inactive {
            self.with_inactive_panel(|app| app.setup_recycle_bin_view());
        }
        if refresh_active || refresh_inactive {
            self.sync_to_tab();
            ctx.request_repaint();
        }
    }

    fn has_visible_recycle_bin_view(&self) -> bool {
        self.navigation_state.is_recycle_bin_view
            || (self.dual_panel_enabled
                && self
                    .dual_panel_inactive_state
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.is_recycle_bin_view))
    }
}

fn recycle_bin_item_to_file_entry(item: RecycleBinItem) -> FileEntry {
    let path = if !item.physical_path.as_os_str().is_empty() {
        item.physical_path
    } else if item.is_directory {
        PathBuf::from("C:\\folder")
    } else if !item.extension.is_empty() {
        PathBuf::from(format!("dummy{}", item.extension))
    } else {
        item.original_path.clone()
    };

    FileEntry {
        path,
        name: item.name,
        is_dir: item.is_directory,
        size: item.size,
        modified: item.date_deleted_unix,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden: false,
        recycle_bin: Some(Box::new(RecycleBinMeta {
            deletion_date: item.date_deleted,
            original_path: item.original_path,
        })),
    }
}
