//! Folder loading: load_folder, filter_items, sort_items, refresh
//!
//! This module handles scanning folders, filtering results, sorting, and manual refresh triggers.

use std::path::PathBuf;

use crate::app::state::ImageViewerApp;
// DISABLED: Prefetch imports (testing HDD I/O impact)
// use crate::workers::idle_warmup::IdleWarmupMessage;
// use crate::workers::predictive_prefetch::PredictiveMessage;
// use crate::workers::prefetch_worker::PrefetchMessage;

mod folder_scan;
mod guards;
mod load_pipeline;
mod refresh;
mod view_updates;

impl ImageViewerApp {
    pub fn load_folder(&mut self, force_refresh: bool) {
        if self.should_skip_folder_load(force_refresh) {
            return;
        }
        self.mark_folder_load_started(force_refresh);
        self.bump_folder_load_generation();

        let _current_path_buf = PathBuf::from(&self.navigation_state.current_path);
        // DISABLED: Predictive prefetch and idle warmup (testing HDD I/O impact)
        // let _ = self
        //     .predictive_sender
        //     .send(PredictiveMessage::NavigatedTo(current_path_buf.clone()));
        // let history_paths: Vec<PathBuf> = self
        //     .navigation
        //     .paths
        //     .iter()
        //     .rev()
        //     .take(5)
        //     .filter(|p| p.len() >= 2 && p.chars().nth(1) == Some(':'))
        //     .map(PathBuf::from)
        //     .collect();
        // if !history_paths.is_empty() {
        //     let _ = self
        //         .predictive_sender
        //         .send(PredictiveMessage::HistoryUpdated(history_paths));
        // }
        // let _ = self
        //     .idle_warmup_sender
        //     .send(IdleWarmupMessage::CurrentDirectory(current_path_buf));

        self.reset_folder_loading_state(force_refresh);

        self.start_folder_load_pipeline(force_refresh);
    }
}
