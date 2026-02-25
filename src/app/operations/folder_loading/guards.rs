use crate::app::state::ImageViewerApp;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use std::time::Instant;

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

    pub(super) fn bump_folder_load_generation(&mut self) {
        self.generation += 1; // Increment local generation
        self.current_generation
            .store(self.generation, AtomicOrdering::Relaxed); // Sync with workers
    }

    pub(super) fn reset_folder_loading_state(&mut self, force_refresh: bool) {
        // 1. State Cleanup (UI Thread)
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
            self.all_items.clear();
            self.pending_all_items_clear = false;
        } else {
            // Watcher-triggered soft reload: keep old items visible to prevent
            // a blank screen flash. They will be replaced atomically once the
            // first batch of the new generation arrives.
            self.pending_all_items_clear = true;
        }
        self.cache_manager.loading_set.clear(); // Clear only pending requests, keep texture cache
        self.cache_manager.folder_preview_loading.clear(); // Clear folder preview loading
        self.cache_manager.pending_upload_set.clear(); // Clear thumbnails awaiting GPU upload
        self.pending_thumbnails.clear(); // Clear pending thumbnails buffer
        self.loading_icons.clear(); // Clear icon loading requests
        self.loading_extensions.clear(); // Clear extension dedup tracking
        self.failed_icons.clear(); // Retry icon extraction in the new folder generation
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.loading_started_at = Instant::now(); // Track loading start for timeout
        self.total_items = 0;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.last_items_rebuild = Instant::now();
    }
}
