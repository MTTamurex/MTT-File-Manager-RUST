//! Async message processing from workers
//!
//! This module processes incoming messages from various background workers
//! (filesystem events, thumbnails, folder sizes, etc.) and updates the UI state.

use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => {{ log::debug!($($arg)*); }}
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        ()
    }};
}

mod dual_panel_events;
mod file_op_events;
mod global_search_events;
mod helpers;
mod rebuild_events;
mod thumbnail_events;
mod thumbnail_rebuild;
mod thumbnail_uploads;
mod thumbnail_workers;
mod watcher_events;
mod watcher_legacy;
mod watcher_reload;

impl ImageViewerApp {
    pub fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        let _t_msg_start = Instant::now();

        let mut saw_device_event = false;
        while self.device_event_receiver.try_recv().is_ok() {
            saw_device_event = true;
        }

        if saw_device_event {
            // Drive was inserted/removed: clear all drive icon caches so icons are re-extracted
            self.item_icon_loader.clear_drive_icons();
            // Mounted media can reuse the same drive letter with different capacity/filesystem.
            // Drop cached volume info so the details panel never shows the previous ISO/DVD.
            self.drive_state.clear_cached_drive_info();
            self.drive_state.drive_info_refresh_pending = false;

            // Launch async drive scan (non-blocking)
            self.drive_state.last_drive_refresh = Instant::now();
            self.reload_drive_list_async();
            self.refresh_drive_info_async();

            // Force immediate repaint without waiting for input events
            ctx.request_repaint_after(std::time::Duration::from_millis(0));
        }

        // Apply async rebuild results (filter/sort) from background thread
        self.process_items_rebuild_results(ctx);

        // OneDrive pin completion: background attrib finished, reload for fresh sync status
        if self
            .onedrive_pin_reload_pending
            .swap(false, std::sync::atomic::Ordering::Acquire)
        {
            self.directory_cache.invalidate(&std::path::PathBuf::from(
                &self.navigation_state.current_path,
            ));
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }

        // PERFORMANCE: Precompute normalized current path once for all comparisons
        let current_path_norm =
            Self::normalize_for_match(Path::new(&self.navigation_state.current_path));

        // BLOCKING: Process all available file operation results in batch
        self.process_file_operation_results(&current_path_norm, ctx);

        // 2. CHECK DE AUTO-REFRESH (WATCHER)
        let watcher_perf = self.process_watcher_events_and_auto_reload(&current_path_norm);
        let _t_watcher_start = watcher_perf.watcher_start;
        let _t_drive_events_done = watcher_perf.drive_events_done;
        let _t_auto_reload_done = watcher_perf.auto_reload_done;

        let _t_streaming_done = self.process_streaming_and_thumbnail_events(ctx);

        // GLOBAL SEARCH: Process search results from worker
        self.process_global_search_events();

        // PERF: Log detailed breakdown when process_incoming_messages is slow
        let _t_msg_total = _t_msg_start.elapsed().as_millis();
        if _t_msg_total > 50 {
            log::warn!("[PERF-MSG] TOTAL={}ms | pre_watcher={}ms | watcher_events={}ms | legacy+autoreload={}ms | streaming={}ms | icons+thumbs={}ms",
                _t_msg_total,
                _t_watcher_start.duration_since(_t_msg_start).as_millis(),
                _t_drive_events_done.duration_since(_t_watcher_start).as_millis(),
                _t_auto_reload_done.duration_since(_t_drive_events_done).as_millis(),
                _t_streaming_done.duration_since(_t_auto_reload_done).as_millis(),
                _t_msg_start.elapsed().as_millis().saturating_sub(_t_streaming_done.duration_since(_t_msg_start).as_millis()),
            );
        }
    }

    /// Enqueue paths for disk-cache invalidation (watcher / non-delete use).
    /// The existence guard is applied: if the file still exists on disk the
    /// thumbnail row is kept (protects against CryptoFS transient events).
    pub(crate) fn enqueue_disk_cache_invalidations(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        use crate::app::init_workers::CacheInvalidationEntry;
        let entries: Vec<CacheInvalidationEntry> = paths
            .into_iter()
            .map(|path| CacheInvalidationEntry {
                path,
                force: false,
                rename_to: None,
            })
            .collect();

        if let Err(_err) = self
            .file_operation_state
            .disk_cache_invalidation_sender
            .send(entries)
        {
            debug_log!(
                "[CACHE] Failed to enqueue disk cache invalidations: {:?}",
                _err
            );
        }
    }

    /// Enqueue paths for **forced** disk-cache invalidation.
    /// Skips the existence guard — use for app-initiated deletes and
    /// manual thumbnail refresh where the file may still exist briefly.
    pub(crate) fn enqueue_disk_cache_invalidations_forced(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        use crate::app::init_workers::CacheInvalidationEntry;
        let entries: Vec<CacheInvalidationEntry> = paths
            .into_iter()
            .map(|path| CacheInvalidationEntry {
                path,
                force: true,
                rename_to: None,
            })
            .collect();

        if let Err(_err) = self
            .file_operation_state
            .disk_cache_invalidation_sender
            .send(entries)
        {
            debug_log!(
                "[CACHE] Failed to enqueue forced disk cache invalidations: {:?}",
                _err
            );
        }
    }
}
