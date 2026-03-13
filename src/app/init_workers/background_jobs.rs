use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use std::sync::{mpsc, Arc};

static GC_WORKER_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(in crate::app) fn spawn_startup_drive_info_preload(
    disks_snapshot: Vec<String>,
    tx: mpsc::Sender<Vec<(String, crate::domain::file_entry::DriveInfo)>>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        use crate::domain::file_entry::DriveInfo;
        use crate::infrastructure::windows::get_volume_info;
        let mut results = Vec::new();
        for path in &disks_snapshot {
            let vol = get_volume_info(path);
            let drive_type = crate::infrastructure::windows::detect_drive_type(path);
            results.push((
                path.clone(),
                DriveInfo {
                    file_system: vol.file_system,
                    total_space: vol.total_space,
                    free_space: vol.free_space,
                    drive_type,
                },
            ));
        }
        let _ = tx.send(results);
        ctx.request_repaint();
    });
}

pub(in crate::app) fn spawn_incremental_gc_worker(disk_cache: Arc<ThumbnailDiskCache>) {
    GC_WORKER_RUNNING.store(true, std::sync::atomic::Ordering::Release);
    std::thread::spawn(move || {
        const GC_INITIAL_DELAY_SECS: u64 = 20;
        const GC_ACTIVE_INTERVAL_SECS: u64 = 180;
        const GC_IDLE_INTERVAL_SECS: u64 = 20;
        const GC_ACTIVE_BATCH: usize = 120;
        const GC_IDLE_BATCH: usize = 600;
        const GC_VACUUM_THRESHOLD: usize = 8_000;

        fn sleep_until_next_cycle(total_secs: u64) -> bool {
            for _ in 0..total_secs {
                if !GC_WORKER_RUNNING.load(std::sync::atomic::Ordering::Acquire) {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            GC_WORKER_RUNNING.load(std::sync::atomic::Ordering::Acquire)
        }

        if !sleep_until_next_cycle(GC_INITIAL_DELAY_SECS) {
            return;
        }

        let mut removed_since_vacuum = 0usize;
        while GC_WORKER_RUNNING.load(std::sync::atomic::Ordering::Acquire) {
            let is_idle_window = crate::infrastructure::onedrive::is_app_minimized();
            let batch = if is_idle_window {
                GC_IDLE_BATCH
            } else {
                GC_ACTIVE_BATCH
            };

            let removed = disk_cache.garbage_collect_incremental(batch);
            if removed > 0 {
                removed_since_vacuum = removed_since_vacuum.saturating_add(removed);
            }

            if is_idle_window
                && removed_since_vacuum >= GC_VACUUM_THRESHOLD
                && disk_cache.run_vacuum()
            {
                log::info!(
                    "[GC] VACUUM completed after removing {} entries",
                    removed_since_vacuum
                );
                removed_since_vacuum = 0;
            }

            let sleep_secs = if is_idle_window {
                GC_IDLE_INTERVAL_SECS
            } else {
                GC_ACTIVE_INTERVAL_SECS
            };
            if !sleep_until_next_cycle(sleep_secs) {
                break;
            }
        }
    });
}
