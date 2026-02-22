use crate::app::state::{ImageViewerApp, ItemsRebuildResult};
use crate::application::sorting;
use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const INLINE_REBUILD_THRESHOLD: usize = 256;
const REBUILD_THROTTLE_MS: u64 = 80;
const REBUILD_PENDING_THRESHOLD: usize = 1200;
const MAX_EAGER_FOLDER_PREVIEWS: usize = 80;

impl ImageViewerApp {
    fn build_sorted_items_snapshot(&self) -> Vec<crate::domain::file_entry::FileEntry> {
        let mut result_items = match sorting::filter_items_opt(&self.all_items, &self.search_query)
        {
            Some(filtered) => filtered,
            None => {
                let mut all = self.all_items.clone();
                sorting::sort_items(
                    &mut all,
                    self.sort_mode,
                    self.sort_descending,
                    self.folders_position,
                );
                all
            }
        };
        if !self.search_query.is_empty() {
            sorting::sort_items(
                &mut result_items,
                self.sort_mode,
                self.sort_descending,
                self.folders_position,
            );
        }
        result_items
    }

    fn spawn_items_rebuild_job(&mut self) {
        self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
        let request_id = self.items_rebuild_request_id;
        let generation = self.generation;
        let items = self.all_items.clone();
        let query = self.search_query.clone();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let folders_position = self.folders_position;
        let sender = self.items_rebuild_sender.clone();

        std::thread::spawn(move || {
            let mut result_items = match sorting::filter_items_opt(&items, &query) {
                Some(filtered) => filtered,
                None => {
                    let mut all = items;
                    sorting::sort_items(&mut all, sort_mode, sort_descending, folders_position);
                    all
                }
            };
            if !query.is_empty() {
                sorting::sort_items(
                    &mut result_items,
                    sort_mode,
                    sort_descending,
                    folders_position,
                );
            }
            let total = result_items.len();
            let _ = sender.send(ItemsRebuildResult {
                generation,
                request_id,
                items: result_items,
                total_items: total,
            });
        });
    }

    fn enqueue_onedrive_eager_folder_previews(&mut self) {
        if !matches!(self.view_mode, crate::domain::file_entry::ViewMode::Grid)
            || self.navigation_state.is_recycle_bin_view
            || !crate::infrastructure::onedrive::is_onedrive_path(&PathBuf::from(
                &self.navigation_state.current_path,
            ))
        {
            return;
        }

        let eager_paths: Vec<PathBuf> = self
            .all_items
            .iter()
            .filter(|i| i.is_dir && !i.is_archive())
            .map(|i| i.path.clone())
            .take(MAX_EAGER_FOLDER_PREVIEWS)
            .collect();

        let mut queued = 0usize;
        for path in eager_paths {
            if self.cache_manager.has_folder_preview(&path)
                || self.cache_manager.is_folder_preview_loading(&path)
            {
                continue;
            }
            self.request_folder_preview_load(path);
            queued += 1;
        }

        if queued > 0 {
            log::debug!(
                "[PERF] OneDrive eager folder preview queue: {} folders",
                queued
            );
        }
    }

    pub(super) fn handle_items_after_end_of_load(&mut self, ctx: &egui::Context) {
        self.is_loading_folder = false;
        self.file_operation_state.pending_deletions.clear();
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;

        // If the deferred clear was never consumed (e.g., empty folder),
        // apply it now so stale items don't leak into the final snapshot.
        if self.pending_all_items_clear {
            self.all_items.clear();
            self.pending_all_items_clear = false;
        }

        if self.all_items.len() <= INLINE_REBUILD_THRESHOLD {
            let result_items = self.build_sorted_items_snapshot();
            self.items = Arc::new(result_items);
            self.total_items = self.items.len();

            if let Some(target_path) = self.pending_select_path.take() {
                let _ = self.select_item_by_path(&target_path);
            }

            log::debug!(
                "[PERF] Inline items rebuild (end-of-load): {} items",
                self.total_items
            );
        } else {
            self.spawn_items_rebuild_job();
        }

        self.enqueue_onedrive_eager_folder_previews();
        self.last_items_rebuild = Instant::now();
        ctx.request_repaint();
    }

    pub(super) fn maybe_schedule_stream_items_rebuild(&mut self, ctx: &egui::Context) {
        if !self.pending_items_rebuild {
            return;
        }

        let elapsed = self.last_items_rebuild.elapsed();
        if elapsed <= Duration::from_millis(REBUILD_THROTTLE_MS)
            && self.pending_items_count < REBUILD_PENDING_THRESHOLD
        {
            return;
        }

        self.spawn_items_rebuild_job();
        self.last_items_rebuild = Instant::now();
        self.pending_items_count = 0;
        self.pending_items_rebuild = false;
        ctx.request_repaint();
    }
}
