use eframe::egui;

use super::init_workers::{spawn_incremental_gc_worker, spawn_startup_drive_info_preload};
use super::state::ImageViewerApp;

impl ImageViewerApp {
    pub(crate) fn run_post_startup_jobs(&mut self, ctx: &egui::Context) {
        let start = std::time::Instant::now();
        self.watch_current_folder();

        let disks_snapshot: Vec<String> = self
            .drive_state
            .disks
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        spawn_startup_drive_info_preload(
            disks_snapshot,
            self.drive_state.drive_info_tx.clone(),
            ctx.clone(),
        );

        spawn_incremental_gc_worker(self.disk_cache.clone(), self.app_state_db.clone());
        log::info!(
            "[STARTUP] post-startup jobs scheduled elapsed_ms={}",
            start.elapsed().as_millis()
        );
    }
}
