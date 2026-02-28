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

mod file_op_events;
mod global_search_events;
mod helpers;
mod rebuild_events;
mod thumbnail_events;
mod thumbnail_rebuild;
mod thumbnail_uploads;
mod thumbnail_workers;
mod watcher_drive_processing;
mod watcher_events;
mod watcher_legacy;
mod watcher_reload;

impl ImageViewerApp {
    pub fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        let _t_msg_start = Instant::now();

        // 1. CHECK DE REFRESH MANUAL (F5)
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.trigger_manual_refresh();
        }

        let mut saw_device_event = false;
        while self.device_event_receiver.try_recv().is_ok() {
            saw_device_event = true;
        }

        if saw_device_event {
            // Drive was inserted/removed: clear all drive icon caches so icons are re-extracted
            self.item_icon_loader.clear_drive_icons();

            // Launch async drive scan (non-blocking)
            self.drive_state.last_drive_refresh = Instant::now();
            self.reload_drive_list_async();

            // Force immediate repaint without waiting for input events
            ctx.request_repaint_after(std::time::Duration::from_millis(0));
        }

        // Apply async rebuild results (filter/sort) from background thread
        self.process_items_rebuild_results(ctx);

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

    fn enqueue_disk_cache_invalidations(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }

        if let Err(_err) = self
            .file_operation_state
            .disk_cache_invalidation_sender
            .send(paths)
        {
            debug_log!(
                "[CACHE] Failed to enqueue disk cache invalidations: {:?}",
                _err
            );
        }
    }
}
