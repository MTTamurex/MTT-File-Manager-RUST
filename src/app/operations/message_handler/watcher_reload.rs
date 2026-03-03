use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn apply_watcher_reload_policy(&mut self) {
        // Execute reload only when debounce allows
        // SUPPRESS auto-reload while file operations are in progress to prevent
        // screen flashing (watcher fires repeatedly as files grow during copy)
        // Skip auto-reload if smart delete already updated the UI
        if self.skip_next_auto_reload {
            self.skip_next_auto_reload = false;
            self.pending_auto_reload = false;
            #[cfg(debug_assertions)]
            log::debug!("[DEBUG] Skipping auto-reload - UI already updated by smart delete");
        }

        // NOTE: Inactivity recovery cooldown removed - no longer needed.
        // The DriveWatcher thread now coalesces and deduplicates events internally
        // (200ms batches, max 500 unique events per batch), so event floods from
        // OneDrive dehydration are absorbed before reaching the UI thread.
        if self.pending_auto_reload
            && self.file_operation_state.file_ops_in_progress == 0
            && !self.is_loading_folder
        {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > Duration::from_millis(theme::AUTO_RELOAD_MS) {
                #[cfg(debug_assertions)]
                log::debug!(
                    "[DEBUG] Checking auto-reload for path: '{}'",
                    self.navigation_state.current_path
                );
                // SKIP for special views (Recycle Bin/Computer) which are managed manually via events
                if self.navigation_state.is_recycle_bin_view
                    || self.navigation_state.is_computer_view
                {
                    self.pending_auto_reload = false;
                } else {
                    // FIX: Removed blocking is_dir() check on the UI thread.
                    // GetFileAttributesW (used by is_dir) can block indefinitely on
                    // network/cloud/USB drives, causing the app to freeze.
                    // If the folder was deleted, the DriveWatcher DELETE event
                    // already fired handle_drive_deleted_event() which calls
                    // navigate_to_nearest_valid_ancestor(). load_folder() itself
                    // handles missing folders gracefully via the loading pipeline.
                    #[cfg(debug_assertions)]
                    log::debug!(
                        "[DEBUG] Auto-reloading with force_refresh=false (watcher-triggered)."
                    );
                    // PERFORMANCE: Use force_refresh=false for watcher-triggered reloads.
                    // force_refresh=true clears ALL caches (textures, thumbnails, folder covers),
                    // empties the items list, and causes a white screen on HDD while rescanning.
                    // With false: directory_cache was already invalidated by watcher events above,
                    // so fresh data is loaded from disk, but texture/thumbnail caches are preserved.
                    // force_refresh=true is reserved for manual refresh (F5) only.
                    self.loaded_path.clear();
                    self.load_folder(false);
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }
    }
}
