use crate::app::state::{ImageViewerApp, ItemsRebuildResult};
use crate::application::sorting;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const INLINE_REBUILD_THRESHOLD: usize = 256;
const REBUILD_THROTTLE_MS: u64 = 80;
const REBUILD_PENDING_THRESHOLD: usize = 1200;
const MAX_EAGER_FOLDER_PREVIEWS: usize = 80;
const MAX_EAGER_NON_USN_FOLDER_COVER_REVALIDATIONS: usize = 96;

impl ImageViewerApp {
    fn has_relevant_stale_visual_state(&self, path: &PathBuf) -> bool {
        self.cache_manager.has_thumbnail(path)
            || self.cache_manager.has_rgba_data(path)
            || self.cache_manager.is_failed(path)
            || self.cache_manager.has_folder_preview(path)
    }

    pub(super) fn capture_stale_items_snapshot(&mut self) {
        if self.stale_items_snapshot.is_some() {
            return;
        }

        let snapshot: HashMap<PathBuf, (u64, u64)> = self
            .all_items
            .iter()
            .filter_map(|item| {
                self.has_relevant_stale_visual_state(&item.path)
                    .then_some((item.path.clone(), (item.modified, item.size)))
            })
            .collect();

        if !snapshot.is_empty() {
            self.stale_items_snapshot = Some(snapshot);
        }
    }

    pub(super) fn reconcile_stale_visual_caches(&mut self) {
        let Some(old_snapshot) = self.stale_items_snapshot.take() else {
            return;
        };

        let new_paths: std::collections::HashSet<&std::path::PathBuf> =
            self.all_items.iter().map(|item| &item.path).collect();

        for item in self.all_items.iter() {
            if let Some(&(old_modified, old_size)) = old_snapshot.get(&item.path) {
                if item.modified != old_modified || item.size != old_size {
                    self.cache_manager.texture_cache.pop(&item.path);
                    self.cache_manager.pop_rgba_data(&item.path);
                    self.cache_manager.failed_thumbnails.pop(&item.path);
                }
            }
        }

        for (old_path, _) in &old_snapshot {
            if !new_paths.contains(old_path) {
                self.cache_manager.texture_cache.pop(old_path);
                self.cache_manager.pop_rgba_data(old_path);
                self.cache_manager.failed_thumbnails.pop(old_path);
                self.cache_manager.invalidate_folder_preview(old_path);
            }
        }
    }

    pub(super) fn should_run_pending_items_rebuild(&self) -> bool {
        let elapsed = self.last_items_rebuild.elapsed();
        elapsed > Duration::from_millis(REBUILD_THROTTLE_MS)
            || self.pending_items_count >= REBUILD_PENDING_THRESHOLD
    }

    fn hydrate_current_folder_modified_hint_after_load(&mut self) {
        if self.navigation_state.is_computer_view || self.navigation_state.is_recycle_bin_view {
            return;
        }

        if self
            .current_folder_modified_hint
            .as_ref()
            .is_some_and(|(path, modified)| {
                path == &PathBuf::from(&self.navigation_state.current_path) && *modified > 0
            })
        {
            return;
        }

        let current_path = PathBuf::from(&self.navigation_state.current_path);
        let Ok(meta) = std::fs::metadata(&current_path) else {
            return;
        };
        let Ok(modified_time) = meta.modified() else {
            return;
        };
        let Ok(duration) = modified_time.duration_since(std::time::UNIX_EPOCH) else {
            return;
        };

        let secs = duration.as_secs();
        if secs == 0 {
            return;
        }

        self.current_folder_modified_hint = Some((current_path.clone(), secs));
        self.folder_modified_hints.put(current_path, secs);
    }

    fn build_sorted_items_snapshot(&self) -> Vec<crate::domain::file_entry::FileEntry> {
        let mut result_items = match sorting::filter_items_opt(&self.all_items, &self.search_query)
        {
            Some(filtered) => filtered,
            None => {
                let mut all = self.all_items.as_ref().clone();
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
        if self.items_rebuild_in_flight {
            return;
        }

        self.items_rebuild_request_id = self.items_rebuild_request_id.wrapping_add(1);
        let request_id = self.items_rebuild_request_id;
        let generation = self.generation;
        let items = self.all_items.clone();
        let query = self.search_query.clone();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let folders_position = self.folders_position;
        let sender = self.items_rebuild_sender.clone();
        let ui_ctx = self.ui_ctx.clone();

        self.items_rebuild_in_flight = true;

        std::thread::spawn(move || {
            let mut result_items = match sorting::filter_items_opt(&items, &query) {
                Some(filtered) => filtered,
                None => {
                    let mut all = match Arc::try_unwrap(items) {
                        Ok(all) => all,
                        Err(shared) => shared.as_ref().clone(),
                    };
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
            ui_ctx.request_repaint();
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

    fn enqueue_non_usn_eager_folder_cover_revalidation(&mut self) {
        if !self.watcher_fallback_polling
            || self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            return;
        }

        let mut queued = 0usize;

        for folder_path in self
            .all_items
            .iter()
            .filter(|item| item.is_dir && !item.is_archive())
            .map(|item| item.path.clone())
            .take(MAX_EAGER_NON_USN_FOLDER_COVER_REVALIDATIONS)
        {
            let _ = self.cover_worker_sender.send(folder_path);
            queued += 1;
        }

        if queued > 0 {
            log::debug!(
                "[PERF] Non-USN eager folder cover revalidation queued: {} folders",
                queued
            );
        }
    }

    pub(super) fn handle_items_after_end_of_load(&mut self, ctx: &egui::Context) {
        self.is_loading_folder = false;
        self.file_operation_state.pending_deletions.clear();
        self.invalidate_active_items_rebuild();
        self.hydrate_current_folder_modified_hint_after_load();

        // If the deferred clear was never consumed (e.g., empty folder),
        // apply it now so stale items don't leak into the final snapshot.
        if self.pending_all_items_clear {
            self.all_items_mut().clear();
            self.pending_all_items_clear = false;
        }

        // Reconcile old vs new items to evict stale textures.
        // Without this, watcher-triggered reloads (force_refresh=false) preserve
        // old GPU textures for items whose content changed on disk, causing the
        // UI to show the wrong thumbnail until the texture is LRU-evicted.
        self.reconcile_stale_visual_caches();

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
        self.enqueue_non_usn_eager_folder_cover_revalidation();
        self.last_items_rebuild = Instant::now();
        ctx.request_repaint();
    }

    pub(super) fn maybe_schedule_stream_items_rebuild(&mut self, ctx: &egui::Context) {
        if !self.pending_items_rebuild {
            return;
        }

        if !self.should_run_pending_items_rebuild() {
            return;
        }

        if self.items_rebuild_in_flight {
            ctx.request_repaint_after(Duration::from_millis(REBUILD_THROTTLE_MS));
            return;
        }

        self.spawn_items_rebuild_job();
        self.last_items_rebuild = Instant::now();
        self.clear_pending_items_rebuild_flags();
        ctx.request_repaint();
    }
}
