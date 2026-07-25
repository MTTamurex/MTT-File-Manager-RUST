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

        spawn_incremental_gc_worker(
            self.disk_cache.clone(),
            self.app_state_db.clone(),
            self.tag_assignment_gc_sender.clone(),
            ctx.clone(),
        );

        // Detect ISO images that were already mounted before the app started.
        let (iso_tx, iso_rx) = std::sync::mpsc::channel();
        self.file_operation_state.iso_detect_rx = Some(iso_rx);
        let repaint_ctx = ctx.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("iso-detect".into())
            .spawn(move || {
                let detected = crate::infrastructure::windows::detect_pre_mounted_isos();
                let _ = iso_tx.send(detected);
                repaint_ctx.request_repaint();
            })
        {
            log::warn!("[ISO-DETECT] Failed to spawn detection worker: {error}");
        }

        log::info!(
            "[STARTUP] post-startup jobs scheduled elapsed_ms={}",
            start.elapsed().as_millis()
        );
    }
}
