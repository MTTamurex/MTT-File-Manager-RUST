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

fn should_skip_exists_guard(path: &std::path::Path) -> bool {
    crate::infrastructure::onedrive::is_onedrive_path(path)
        || crate::infrastructure::io_priority::is_network_or_virtual(path)
}

pub(in crate::app) fn spawn_disk_cache_invalidation_worker(
    disk_cache: Arc<ThumbnailDiskCache>,
) -> mpsc::Sender<Vec<CacheInvalidationEntry>> {
    let (disk_cache_invalidation_tx, disk_cache_invalidation_rx) =
        mpsc::channel::<Vec<CacheInvalidationEntry>>();
    let disk_cache_for_invalidation = disk_cache.clone();
    std::thread::Builder::new()
        .name("disk-cache-invalidation".into())
        .spawn(move || {
        while let Ok(entries) = disk_cache_invalidation_rx.recv() {
            let mut unique_paths = std::collections::HashSet::with_capacity(entries.len());
            for entry in entries {
                if unique_paths.insert(entry.path.clone()) {
                    if entry.force {
                        // App-initiated delete/refresh: unconditionally remove
                        // all cache rows. The Shell may not have finished yet,
                        // so `fast_path_exists` would give a false positive.
                        disk_cache_for_invalidation.remove_cache_for_path(&entry.path);
                    } else if should_skip_exists_guard(entry.path.as_path()) {
                        // BUG FIX: On virtual/network drives we cannot probe
                        // existence safely (GetFileAttributesW can block
                        // indefinitely).  Previously this called
                        // remove_cache_for_path which does
                        //   DELETE FROM thumbnails WHERE path LIKE 'folder\%'
                        // wiping ALL child thumbnails recursively — even though
                        // the invalidation was triggered by a benign cover
                        // worker update, consistency probe, or watcher event.
                        //
                        // Fix: only clear folder visual caches (cover/preview).
                        // Individual file thumbnails are preserved.  True orphans
                        // will be cleaned up by the incremental GC.
                        disk_cache_for_invalidation.remove_folder_preview_cache(&entry.path);
                        disk_cache_for_invalidation.remove_folder_cover(&entry.path);
                        log::debug!(
                            "[CACHE-INVALIDATION] Virtual/network path, cleared folder visual cache only (thumbnails preserved): {:?}",
                            entry.path.file_name().unwrap_or_default()
                        );
                    } else if crate::infrastructure::onedrive::fast_path_exists(entry.path.as_path()) {
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
        })
        .expect("failed to spawn disk-cache-invalidation worker");
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

            // Fast path: try the search service for NTFS volumes (in-memory, zero I/O).
            if is_ntfs_volume(&folder_path) {
                match crate::infrastructure::global_search::folder_size(&folder_path) {
                    Ok((total_size, file_count)) => {
                        log::info!(
                            "[FOLDER-SIZE] IPC complete path={} total_gb={:.2} files={}",
                            folder_path.display(),
                            total_size as f64 / 1_073_741_824.0,
                            file_count,
                        );
                        let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                            folder_path: folder_path.clone(),
                            total_size,
                        });
                        folder_size_ctx.request_repaint();
                        continue;
                    }
                    Err(e) => {
                        // Service not available or sizes not loaded — fall through
                        // to classic FindFirstFileExW scan.
                        log::info!(
                            "[FOLDER-SIZE] IPC failed path={} falling_back=true reason={}",
                            folder_path.display(),
                            e
                        );
                    }
                }
            }

            // Fallback: classic FindFirstFileExW parallel scan (for non-NTFS
            // or when the search service is unavailable).
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
                    log::info!(
                        "[FOLDER-SIZE] Fallback complete path={} total_gb={:.2}",
                        folder_path.display(),
                        total_size as f64 / 1_073_741_824.0,
                    );
                    let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                        folder_path,
                        total_size,
                    });
                }
                None => {
                    log::info!(
                        "[FOLDER-SIZE] Fallback cancelled path={}",
                        folder_path.display(),
                    );
                    let _ = folder_size_res_tx.send(FolderSizeMessage::Cancelled { folder_path });
                }
            }
            folder_size_ctx.request_repaint();
            crate::infrastructure::io_priority::reset_thread_priority();
        }
    });

    (folder_size_req_tx, folder_size_res_rx, folder_size_cancel)
}

/// Check if a path resides on an NTFS filesystem.
/// Uses `GetVolumeInformationW` with the drive root.
/// Returns `false` on any error (safe default — triggers fallback to FindFirstFileExW).
fn is_ntfs_volume(path: &std::path::Path) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    // Extract drive root: "C:\"
    let root = match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            let s = prefix.as_os_str().to_string_lossy();
            if s.len() >= 2 && s.as_bytes()[1] == b':' {
                format!("{}\\", &s[..2])
            } else {
                return false;
            }
        }
        _ => return false,
    };

    let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    let mut fs_name = [0u16; 16];

    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(root_wide.as_ptr()),
            None,
            None,
            None,
            None,
            Some(&mut fs_name),
        )
    };

    if ok.is_err() {
        return false;
    }

    let fs = String::from_utf16_lossy(&fs_name)
        .trim_end_matches('\0')
        .to_ascii_uppercase();
    fs == "NTFS"
}
