use eframe::egui;

use super::init_workers::{spawn_incremental_gc_worker, spawn_startup_drive_info_preload};
use super::state::ImageViewerApp;

pub(in crate::app) fn run_post_startup_jobs(app: &mut ImageViewerApp, ctx: &egui::Context) {
    app.watch_current_folder();

    let disks_snapshot: Vec<String> = app
        .drive_state
        .disks
        .iter()
        .map(|(p, _)| p.clone())
        .collect();
    spawn_startup_drive_info_preload(
        disks_snapshot,
        app.drive_state.drive_info_tx.clone(),
        ctx.clone(),
    );

    spawn_incremental_gc_worker(app.disk_cache.clone(), app.app_state_db.clone());
}
