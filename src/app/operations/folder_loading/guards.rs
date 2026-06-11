use crate::app::state::ImageViewerApp;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Instant;

/// Global monotonic counter for folder-load generation IDs.
///
/// Using a single global counter guarantees that each panel (active or
/// inactive) receives a **unique** generation on every load, preventing
/// the batch-routing logic in `process_streaming_and_thumbnail_events`
/// from mis-routing results when both panels happen to share the same
/// generation value.
static GLOBAL_GENERATION: AtomicUsize = AtomicUsize::new(0);

/// Return the next globally-unique generation ID (1, 2, 3, …).
/// Value `0` is reserved as the "never loaded" sentinel.
fn next_generation() -> usize {
    GLOBAL_GENERATION.fetch_add(1, AtomicOrdering::Relaxed) + 1
}

impl ImageViewerApp {
    pub(super) fn should_skip_folder_load(&self, force_refresh: bool) -> bool {
        // GUARD CLAUSE: Prevent spam by checking if we're already on this path
        log::debug!(
            "[GUARD] Checking load_folder: current_path={:?}, loaded_path={:?}, force_refresh={}",
            self.navigation_state.current_path,
            self.loaded_path,
            force_refresh
        );

        if !force_refresh && self.navigation_state.current_path == self.loaded_path {
            log::debug!(
                "[GUARD] Skipping load_folder for {:?} - already loaded",
                self.navigation_state.current_path
            );
            return true;
        }

        false
    }

    pub(super) fn mark_folder_load_started(&mut self, force_refresh: bool) {
        log::debug!(
            "[GUARD] load_folder called for {:?} (force_refresh={}, loaded_path={:?})",
            self.navigation_state.current_path,
            force_refresh,
            self.loaded_path
        );

        // Mark as loaded immediately to prevent spam.
        self.loaded_path = self.navigation_state.current_path.clone();

        log::debug!(
            "[GUARD] Starting folder loading process for {:?}",
            self.navigation_state.current_path
        );
    }

    pub(in crate::app::operations) fn bump_folder_load_generation(&mut self) {
        self.generation = next_generation(); // Globally unique generation ID
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed); // Sync with workers
    }

    pub(super) fn reset_folder_loading_state(&mut self, force_refresh: bool, trim_icons: bool) {
        // 1. State Cleanup (UI Thread)
        let preserve_visual_pipeline = !force_refresh && !self.items.is_empty();
        let preserve_inactive_dual_panel_pipeline =
            !force_refresh && self.should_preserve_inactive_dual_panel_thumbnail_pipeline();

        if force_refresh {
            self.cache_manager.texture_cache.clear();
            self.cache_manager.folder_preview_cache.clear();
            self.cache_manager.failed_thumbnails.clear();
            crate::workers::thumbnail::clear_all_failures();
            self.directory_cache.clear();
        }

        if force_refresh {
            // Hard refresh (F5): clear everything immediately.
            self.items = Arc::new(Vec::new());
            self.all_items_mut().clear();
            self.pending_all_items_clear = false;
            self.hold_visible_items_until_load_complete = false;
        } else {
            // Watcher-triggered soft reload: keep old items visible to prevent
            // a blank/partial-list flash. They will be replaced atomically once
            // the new generation reaches end-of-load.
            self.pending_all_items_clear = true;
            self.hold_visible_items_until_load_complete = preserve_visual_pipeline;
        }
        // Soft reloads of the same folder can preserve icon caches to avoid a
        // per-file icon re-extraction storm for executables (.exe/.lnk/.ico).
        if !preserve_visual_pipeline {
            if preserve_inactive_dual_panel_pipeline {
                self.prune_thumbnail_pipeline_for_dual_panel_navigation("folder-load");
            } else {
                self.discard_thumbnail_pipeline_for_navigation("folder-load", trim_icons);
                self.loading_icons.clear(); // Clear icon loading requests
                self.loading_extensions.clear(); // Clear extension dedup tracking
                self.failed_icons.clear(); // Retry icon extraction in the new folder generation
                self.scanned_folders.clear();
            }
        }
        if !preserve_visual_pipeline {
            self.selected_item = None;
            self.total_items = 0;
        }
        self.is_loading_folder = true;
        self.loading_started_at = Instant::now(); // Track loading start for timeout
        self.invalidate_active_items_rebuild();
    }
}
