use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::icon_disk_cache::IconDiskCache;
use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};

static GC_WORKER_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(crate) fn stop_gc_worker() {
    GC_WORKER_RUNNING.store(false, Ordering::Relaxed);
}

pub(in crate::app) fn spawn_startup_drive_info_preload(
    disks_snapshot: Vec<String>,
    tx: mpsc::Sender<Vec<(String, crate::domain::file_entry::DriveInfo)>>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        use crate::domain::file_entry::DriveInfo;
        use crate::infrastructure::windows::{get_volume_info, query_hardware_fields};
        let mut results = Vec::new();
        for path in &disks_snapshot {
            let vol = get_volume_info(path);
            let drive_type = crate::infrastructure::windows::detect_drive_type(path);
            let hw = query_hardware_fields(path, drive_type);
            results.push((
                path.clone(),
                DriveInfo {
                    file_system: vol.file_system,
                    total_space: vol.total_space,
                    free_space: vol.free_space,
                    drive_type,
                    model: hw.model,
                    serial_number: hw.serial_number,
                    firmware_revision: hw.firmware_revision,
                    bus_type: hw.bus_type,
                },
            ));
        }
        let _ = tx.send(results);
        ctx.request_repaint();
    });
}

pub(in crate::app) fn spawn_incremental_gc_worker(
    disk_cache: Arc<ThumbnailDiskCache>,
    app_state_db: Arc<AppStateDb>,
    tag_gc_sender: mpsc::Sender<crate::app::operations::tag_ops::TagPathUpdate>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        // Delayed to avoid competing for a cold NTFS cache during the user's
        // first interactions (typically selecting a tag with many items).
        // The very first tag selection after restart / long idle is the
        // worst case (cold OS file cache) and the GC's path_exists_fast
        // calls would steal disk bandwidth from the GetFileAttributesExW
        // loop in setup_tag_view, so we yield the first ~45s.
        const GC_INITIAL_DELAY_SECS: u64 = 45;
        const GC_ACTIVE_INTERVAL_SECS: u64 = 180;
        const GC_IDLE_INTERVAL_SECS: u64 = 20;
        const GC_ACTIVE_BATCH: usize = 120;
        const GC_IDLE_BATCH: usize = 600;
        const GC_VACUUM_THRESHOLD: usize = 8_000;

        fn sleep_until_next_cycle(total_secs: u64) -> bool {
            for _ in 0..total_secs {
                if !GC_WORKER_RUNNING.load(Ordering::Relaxed) {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            GC_WORKER_RUNNING.load(Ordering::Relaxed)
        }

        if !sleep_until_next_cycle(GC_INITIAL_DELAY_SECS) {
            return;
        }

        let mut removed_since_vacuum = 0usize;
        while GC_WORKER_RUNNING.load(Ordering::Relaxed) {
            let is_idle_window = crate::infrastructure::onedrive::is_app_minimized();
            let batch = if is_idle_window {
                GC_IDLE_BATCH
            } else {
                GC_ACTIVE_BATCH
            };

            let removed = disk_cache.garbage_collect_incremental(batch);
            let removed_covers = app_state_db.garbage_collect_covers_incremental(batch);
            let (removed_tags, removed_tag_paths) =
                app_state_db.garbage_collect_tag_assignments_incremental(batch);
            if !removed_tag_paths.is_empty() {
                let _ = tag_gc_sender.send(
                    crate::app::operations::tag_ops::TagPathUpdate::PersistedRemoval(
                        removed_tag_paths,
                    ),
                );
                ctx.request_repaint();
            }
            let total_removed = removed + removed_covers + removed_tags;
            if total_removed > 0 {
                removed_since_vacuum = removed_since_vacuum.saturating_add(total_removed);
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

pub(in crate::app) fn spawn_file_icon_cache_gc_worker(icon_disk_cache: Arc<IconDiskCache>) {
    std::thread::spawn(move || {
        const INITIAL_DELAY_SECS: u64 = 30;
        const ACTIVE_INTERVAL_SECS: u64 = 300;
        const IDLE_INTERVAL_SECS: u64 = 60;

        fn sleep_until_next_cycle(total_secs: u64) -> bool {
            for _ in 0..total_secs {
                if !GC_WORKER_RUNNING.load(Ordering::Relaxed) {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            GC_WORKER_RUNNING.load(Ordering::Relaxed)
        }

        if !sleep_until_next_cycle(INITIAL_DELAY_SECS) {
            return;
        }

        while GC_WORKER_RUNNING.load(Ordering::Relaxed) {
            let _ = icon_disk_cache.garbage_collect_file_icons();
            let sleep_secs = if crate::infrastructure::onedrive::is_app_minimized() {
                IDLE_INTERVAL_SECS
            } else {
                ACTIVE_INTERVAL_SECS
            };
            if !sleep_until_next_cycle(sleep_secs) {
                break;
            }
        }
    });
}
