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
            if !self.pending_auto_reload {
                #[cfg(debug_assertions)]
                log::debug!("[DEBUG] Skipping auto-reload - UI already updated by smart delete");
            } else {
                #[cfg(debug_assertions)]
                log::debug!(
                    "[DEBUG] Smart delete skip ignored because another watcher event already scheduled a reload"
                );
            }
        }

        // NOTE: Inactivity recovery cooldown removed - no longer needed.
        // The DriveWatcher thread now coalesces and deduplicates events internally
        // (200ms batches, max 500 unique events per batch), so event floods from
        // OneDrive dehydration are absorbed before reaching the UI thread.
        // Cooldown after file operations: suppress watcher reloads to avoid
        // status bar flickering when many files were created (e.g. archive extraction).
        if let Some(until) = self.watcher_cooldown_until {
            if Instant::now() < until {
                self.pending_auto_reload = false;
                return;
            }
            self.watcher_cooldown_until = None;
        }

        // EST-01: overflow-coalesced reload. The bounded notify channel
        // dropped events during a sustained storm; regardless of how many
        // were dropped, at most one debounced full reload is applied here,
        // and only while the affected folder is still the current one.
        #[cfg(feature = "notify-watcher")]
        if let Some(overflow_path) = self.watcher_overflow_reload_for.clone() {
            let still_current = Self::normalize_for_match(&overflow_path)
                == Self::normalize_for_match(std::path::Path::new(
                    &self.navigation_state.current_path,
                ));
            if !still_current {
                // Watch target changed since the storm — the marker is stale.
                self.watcher_overflow_reload_for = None;
            } else if self.file_operation_state.file_ops_in_progress == 0
                && !self.layout.saved_is_minimized
                && !self.is_loading_folder
                && !self.navigation_state.is_recycle_bin_view
                && !self.navigation_state.is_computer_view
            {
                self.watcher_overflow_reload_for = None;
                let elapsed = self.last_auto_reload.elapsed();
                if elapsed > Duration::from_millis(theme::AUTO_RELOAD_MS) {
                    self.pending_auto_reload = false;
                    self.loaded_path.clear();
                    log::info!(
                        "[FS-WATCH] Overflow-coalesced reload for {:?} (dropped events during storm)",
                        self.navigation_state.current_path
                    );
                    self.reload_current_folder_preserving_icon_cache();
                    self.last_auto_reload = Instant::now();
                } else {
                    // Debounce window still open — keep the flag for the next cycle.
                    self.watcher_overflow_reload_for = Some(overflow_path);
                    self.ui_ctx
                        .request_repaint_after(Duration::from_millis(theme::AUTO_RELOAD_MS + 25));
                }
            }
            // Otherwise (file ops running, minimized, loading, virtual views):
            // keep the flag and retry on a later frame.
        }

        if self.pending_auto_reload
            && self.file_operation_state.file_ops_in_progress == 0
            && !self.layout.saved_is_minimized
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
                    // If the folder was deleted, explicit notify events and the
                    // consistency probe route the app through
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
                    log::info!(
                        "[FS-WATCH] AUTO-RELOAD triggered for {:?} (elapsed={}ms)",
                        self.navigation_state.current_path,
                        elapsed.as_millis()
                    );
                    self.reload_current_folder_preserving_icon_cache();
                }
                self.last_auto_reload = Instant::now();
                self.pending_auto_reload = false;
            }
        }
    }
}
