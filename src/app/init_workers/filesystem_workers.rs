use crate::app::folder_size_state::FolderSizeMessage;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};

/// Payload for the disk-cache invalidation channel.
/// When `force` is `true` the existence guard is skipped — used for
/// app-initiated deletes where the Shell hasn't finished yet.
pub struct CacheInvalidationEntry {
    pub path: PathBuf,
    pub force: bool,
}

pub(in crate::app) fn spawn_disk_cache_invalidation_worker(
    disk_cache: Arc<ThumbnailDiskCache>,
) -> mpsc::Sender<Vec<CacheInvalidationEntry>> {
    let (disk_cache_invalidation_tx, disk_cache_invalidation_rx) =
        mpsc::channel::<Vec<CacheInvalidationEntry>>();
    let disk_cache_for_invalidation = disk_cache.clone();
    std::thread::spawn(move || {
        while let Ok(entries) = disk_cache_invalidation_rx.recv() {
            let mut unique_paths = std::collections::HashSet::with_capacity(entries.len());
            for entry in entries {
                if unique_paths.insert(entry.path.clone()) {
                    if entry.force {
                        // App-initiated delete/refresh: unconditionally remove
                        // all cache rows. The Shell may not have finished yet,
                        // so `fast_path_exists` would give a false positive.
                        disk_cache_for_invalidation.remove_cache_for_path(&entry.path);
                    } else if crate::infrastructure::onedrive::fast_path_exists(
                        entry.path.as_path(),
                    ) {
                        // Guard: if the path still exists on disk, the DELETE
                        // event was transient (common on FUSE/WinFsp drivers
                        // like Cryptomator that emit DELETE+CREATE during
                        // internal refresh). Keep thumbnail rows intact to avoid
                        // permanent thumbnail loss, but still clear folder visual
                        // caches (cover/preview) so stale UI can refresh.
                        disk_cache_for_invalidation.remove_folder_preview_cache(&entry.path);
                        disk_cache_for_invalidation.remove_folder_cover(&entry.path);
                        log::debug!(
                            "[CACHE-INVALIDATION] Path exists, invalidated folder visual cache only: {:?}",
                            entry.path.file_name().unwrap_or_default()
                        );
                    } else {
                        disk_cache_for_invalidation.remove_cache_for_path(&entry.path);
                    }
                }
            }
        }
    });
    disk_cache_invalidation_tx
}

pub(in crate::app) fn spawn_folder_preview_workers(
    ctx: &egui::Context,
    disk_cache: Arc<ThumbnailDiskCache>,
    folder_composer: Arc<crate::infrastructure::folder_compose::FolderComposer>,
) -> (
    crossbeam_channel::Sender<PathBuf>,
    mpsc::Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,
) {
    // M-18: crossbeam Receiver is Clone + Send + Sync — workers share it directly
    // without the Arc<Mutex<>> serialisation bottleneck.
    let (folder_preview_tx, folder_preview_rx) = crossbeam_channel::unbounded::<PathBuf>();
    let (folder_preview_res_tx, folder_preview_res_rx) = mpsc::channel();

    {
        use crate::workers::folder_preview_worker::spawn_folder_preview_worker;
        let cpu = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let worker_count = cpu.clamp(2, 6);
        for _ in 0..worker_count {
            spawn_folder_preview_worker(
                folder_preview_rx.clone(),
                folder_preview_res_tx.clone(),
                ctx.clone(),
                disk_cache.clone(),
                folder_composer.clone(),
            );
        }
    }

    (folder_preview_tx, folder_preview_res_rx)
}

pub(in crate::app) fn spawn_folder_size_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<PathBuf>,
    mpsc::Receiver<FolderSizeMessage>,
    Arc<AtomicBool>,
) {
    let (folder_size_req_tx, folder_size_req_rx) = mpsc::channel::<PathBuf>();
    let (folder_size_res_tx, folder_size_res_rx) = mpsc::channel::<FolderSizeMessage>();
    let folder_size_ctx = ctx.clone();
    let folder_size_cancel = Arc::new(AtomicBool::new(false));
    let folder_size_cancel_worker = folder_size_cancel.clone();

    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;

        while let Ok(folder_path) = folder_size_req_rx.recv() {
            folder_size_cancel_worker.store(false, Ordering::Release);

            let mut latest_path = folder_path;
            while let Ok(newer_path) = folder_size_req_rx.try_recv() {
                let _ = folder_size_res_tx.send(FolderSizeMessage::Cancelled {
                    folder_path: latest_path,
                });
                latest_path = newer_path;
            }
            let folder_path = latest_path;
            folder_size_cancel_worker.store(false, Ordering::Release);

            let is_ssd = crate::infrastructure::io_priority::is_ssd(&folder_path);
            let priority = if is_ssd {
                crate::infrastructure::io_priority::IOPriority::Prefetch
            } else {
                crate::infrastructure::io_priority::IOPriority::Background
            };
            crate::infrastructure::io_priority::set_thread_priority(priority);

            let cancel_ref = folder_size_cancel_worker.clone();
            let res_tx = folder_size_res_tx.clone();
            let path_clone = folder_path.clone();
            let ctx_clone = folder_size_ctx.clone();

            let result =
                crate::infrastructure::windows::folder_size::calculate_folder_size_parallel(
                    &folder_path,
                    &cancel_ref,
                    move |partial_size| {
                        let _ = res_tx.send(FolderSizeMessage::Progress {
                            folder_path: path_clone.clone(),
                            total_size: partial_size,
                        });
                        ctx_clone.request_repaint();
                    },
                );

            match result {
                Some(total_size) => {
                    let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                        folder_path,
                        total_size,
                    });
                }
                None => {
                    let _ = folder_size_res_tx.send(FolderSizeMessage::Cancelled { folder_path });
                }
            }
            folder_size_ctx.request_repaint();
            crate::infrastructure::io_priority::reset_thread_priority();
        }
    });

    (folder_size_req_tx, folder_size_res_rx, folder_size_cancel)
}
