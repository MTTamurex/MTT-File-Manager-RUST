use crate::app::folder_size_state::FolderSizeMessage;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

const NTFS_FOLDER_SIZE_SERVICE_RETRY_ATTEMPTS: usize = 20;
const NTFS_FOLDER_SIZE_SERVICE_RETRY_MS: u64 = 250;

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

fn is_retryable_folder_size_service_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("sizes not loaded")
        || lower.contains("volume not ready")
        || lower.contains("volume not indexed")
        || lower.contains("all pipe instances are busy")
        || lower.contains("no process is on the other end of the pipe")
        || lower.contains("pipe closed during read")
        || lower.contains("search service timeout")
        || lower.contains("peeknamedpipe failed")
        || lower.contains("readfile failed")
        || lower.contains("writefile failed")
}

fn query_ntfs_folder_size_with_retry(
    folder_path: &std::path::Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(u64, u64), String> {
    let mut last_error = String::from("Search service not available");

    for attempt in 0..NTFS_FOLDER_SIZE_SERVICE_RETRY_ATTEMPTS {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".to_string());
        }

        match crate::infrastructure::global_search::folder_size(folder_path) {
            Ok(result) => return Ok(result),
            Err(error) => {
                let retryable = is_retryable_folder_size_service_error(&error);
                last_error = error;
                if !retryable || attempt + 1 >= NTFS_FOLDER_SIZE_SERVICE_RETRY_ATTEMPTS {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(
                    NTFS_FOLDER_SIZE_SERVICE_RETRY_MS,
                ));
            }
        }
    }

    Err(last_error)
}

fn is_onedrive_folder_size_service_lookup_error(
    folder_path: &std::path::Path,
    message: &str,
) -> bool {
    if !crate::infrastructure::onedrive::is_onedrive_path(folder_path) {
        return false;
    }

    let lower = message.to_ascii_lowercase();
    lower.contains("path not found in index") || lower.contains("invalid path")
}

fn query_onedrive_folder_size_with_timeout(
    folder_path: &std::path::Path,
    cancel: &Arc<AtomicBool>,
) -> Result<u64, String> {
    fn walk_directory(
        path: &std::path::Path,
        cancel: &Arc<AtomicBool>,
        depth: usize,
    ) -> Result<u64, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".to_string());
        }
        if depth > 256 {
            return Err("onedrive folder-size recursion depth exceeded".to_string());
        }

        match crate::infrastructure::onedrive::onedrive_read_directory(path) {
            crate::infrastructure::onedrive::IoTimeoutResult::Ok(entries) => {
                let mut total_size = 0u64;
                for (name, attrs, size, _modified) in entries {
                    if cancel.load(Ordering::Acquire) {
                        return Err("cancelled".to_string());
                    }

                    let child_path = path.join(name);
                    let is_dir = (attrs & 0x10) != 0;
                    if is_dir {
                        total_size = total_size.saturating_add(walk_directory(
                            &child_path,
                            cancel,
                            depth + 1,
                        )?);
                    } else {
                        total_size = total_size.saturating_add(size);
                    }
                }
                Ok(total_size)
            }
            crate::infrastructure::onedrive::IoTimeoutResult::Timeout => {
                Err("onedrive directory enumeration timeout".to_string())
            }
            crate::infrastructure::onedrive::IoTimeoutResult::Err(kind) => Err(format!(
                "onedrive directory enumeration failed: {:?}",
                kind
            )),
        }
    }

    walk_directory(folder_path, cancel, 0)
}

pub(in crate::app) fn spawn_disk_cache_invalidation_worker(
    disk_cache: Arc<ThumbnailDiskCache>,
    app_state_db: Arc<AppStateDb>,
) -> mpsc::Sender<Vec<CacheInvalidationEntry>> {
    let (disk_cache_invalidation_tx, disk_cache_invalidation_rx) =
        mpsc::channel::<Vec<CacheInvalidationEntry>>();
    let disk_cache_for_invalidation = disk_cache.clone();
    let app_state_for_invalidation = app_state_db.clone();
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
                        app_state_for_invalidation.remove_covers_for_path(&entry.path);
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
                        app_state_for_invalidation.remove_folder_cover(&entry.path);
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
                        app_state_for_invalidation.remove_folder_cover(&entry.path);
                        log::debug!(
                            "[CACHE-INVALIDATION] Path exists, invalidated folder visual cache only: {:?}",
                            entry.path.file_name().unwrap_or_default()
                        );
                    } else {
                        disk_cache_for_invalidation.remove_cache_for_path(&entry.path);
                        app_state_for_invalidation.remove_covers_for_path(&entry.path);
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
    // Bounded(60): inherent backpressure prevents unbounded growth during rapid
    // navigation — preview is best-effort so dropping overflow is safe.
    let (folder_preview_tx, folder_preview_rx) = crossbeam_channel::bounded::<PathBuf>(60);
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

            // NTFS fast path: use the service's indexed subtree total.
            if is_ntfs_volume(&folder_path) {
                match query_ntfs_folder_size_with_retry(&folder_path, &folder_size_cancel_worker) {
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
                        if e == "cancelled" {
                            let _ = folder_size_res_tx.send(FolderSizeMessage::Cancelled {
                                folder_path,
                            });
                            folder_size_ctx.request_repaint();
                            continue;
                        }

                        if is_onedrive_folder_size_service_lookup_error(&folder_path, &e) {
                            match query_onedrive_folder_size_with_timeout(
                                &folder_path,
                                &folder_size_cancel_worker,
                            ) {
                                Ok(total_size) => {
                                    log::info!(
                                        "[FOLDER-SIZE] OneDrive fallback complete path={} total_gb={:.2}",
                                        folder_path.display(),
                                        total_size as f64 / 1_073_741_824.0,
                                    );
                                    let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                                        folder_path: folder_path.clone(),
                                        total_size,
                                    });
                                    folder_size_ctx.request_repaint();
                                    continue;
                                }
                                Err(fallback_error) if fallback_error == "cancelled" => {
                                    let _ = folder_size_res_tx.send(FolderSizeMessage::Cancelled {
                                        folder_path,
                                    });
                                    folder_size_ctx.request_repaint();
                                    continue;
                                }
                                Err(fallback_error) => {
                                    log::warn!(
                                        "[FOLDER-SIZE] OneDrive fallback failed path={} reason={}",
                                        folder_path.display(),
                                        fallback_error,
                                    );
                                }
                            }
                        }

                        // NTFS should be served by the indexed path. Avoid
                        // regressing to a recursive local scan here.
                        log::warn!(
                            "[FOLDER-SIZE] IPC failed path={} giving_up=true reason={}",
                            folder_path.display(),
                            e,
                        );
                        let _ = folder_size_res_tx.send(FolderSizeMessage::Cancelled {
                            folder_path,
                        });
                        folder_size_ctx.request_repaint();
                        continue;
                    }
                }
            }

            // Non-NTFS fallback: local recursive scan.
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

/// Spawns a batch worker for list-view folder sizes.
///
/// Uses the NTFS service fast path when available, falling back to
/// `FindFirstFileExW` only for non-NTFS volumes. The shared `cancel` flag is
/// checked between items and passed through to `calculate_folder_size_parallel`
/// so in-flight slow scans abort promptly on navigation. A generation counter
/// invalidates queued requests from previous folders.
pub(in crate::app) fn spawn_folder_size_batch_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<crate::app::folder_size_state::BatchSizeRequest>,
    mpsc::Receiver<crate::app::folder_size_state::BatchSizeResult>,
    Arc<AtomicBool>,
    Arc<AtomicU64>,
) {
    use crate::app::folder_size_state::{BatchSizeRequest, BatchSizeResult};

    let (batch_tx, batch_rx) = mpsc::channel::<BatchSizeRequest>();
    let (res_tx, res_rx) = mpsc::channel::<BatchSizeResult>();
    let ctx = ctx.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);
    let generation = Arc::new(AtomicU64::new(0));
    let generation_worker = Arc::clone(&generation);

    std::thread::Builder::new()
        .name("folder-size-batch".into())
        .spawn(move || {
            while let Ok((path, req_gen, req_epoch)) = batch_rx.recv() {
                // Skip stale requests from a previous generation (previous folder).
                if req_gen != generation_worker.load(Ordering::Acquire) {
                    continue;
                }

                // Skip stale requests after a navigation cancel.
                if cancel_worker.load(Ordering::Acquire) {
                    continue;
                }

                // Skip empty sentinel paths (sent during cancel drain).
                if path.as_os_str().is_empty() {
                    continue;
                }

                // NTFS fast path: use the service index instead of local recursion.
                if is_ntfs_volume(&path) {
                    match query_ntfs_folder_size_with_retry(&path, &cancel_worker) {
                        Ok((total_size, _file_count)) => {
                            let _ = res_tx.send(BatchSizeResult {
                                folder_path: path,
                                total_size: Some(total_size),
                                request_epoch: req_epoch,
                            });
                            ctx.request_repaint();
                            continue;
                        }
                        Err(error) if error == "cancelled" => {
                            continue;
                        }
                        Err(error) => {
                            if is_onedrive_folder_size_service_lookup_error(&path, &error) {
                                match query_onedrive_folder_size_with_timeout(&path, &cancel_worker)
                                {
                                    Ok(total_size) => {
                                        let _ = res_tx.send(BatchSizeResult {
                                            folder_path: path,
                                            total_size: Some(total_size),
                                            request_epoch: req_epoch,
                                        });
                                        ctx.request_repaint();
                                        continue;
                                    }
                                    Err(fallback_error) if fallback_error == "cancelled" => {
                                        continue;
                                    }
                                    Err(fallback_error) => {
                                        log::warn!(
                                            "[FOLDER-SIZE] Batch OneDrive fallback failed path={} reason={}",
                                            path.display(),
                                            fallback_error,
                                        );
                                    }
                                }
                            }

                            log::warn!(
                                "[FOLDER-SIZE] Batch IPC failed path={} giving_up=true reason={}",
                                path.display(),
                                error,
                            );
                            let _ = res_tx.send(BatchSizeResult {
                                folder_path: path,
                                total_size: None,
                                request_epoch: req_epoch,
                            });
                            ctx.request_repaint();
                            continue;
                        }
                    }
                }

                // Re-check cancel/generation before starting a potentially slow scan.
                if cancel_worker.load(Ordering::Acquire)
                    || req_gen != generation_worker.load(Ordering::Acquire)
                {
                    continue;
                }

                // Slow path: FindFirstFileExW parallel scan.
                let is_ssd = crate::infrastructure::io_priority::is_ssd(&path);
                let priority = if is_ssd {
                    crate::infrastructure::io_priority::IOPriority::Prefetch
                } else {
                    crate::infrastructure::io_priority::IOPriority::Background
                };
                crate::infrastructure::io_priority::set_thread_priority(priority);

                let result =
                    crate::infrastructure::windows::folder_size::calculate_folder_size_parallel(
                        &path,
                        &cancel_worker,
                        |_partial| { /* no progress needed for list view */ },
                    );

                // Only send if scan completed (not cancelled).
                if !cancel_worker.load(Ordering::Acquire) {
                    if let Some(total_size) = result {
                        let _ = res_tx.send(BatchSizeResult {
                            folder_path: path,
                            total_size: Some(total_size),
                            request_epoch: req_epoch,
                        });
                        ctx.request_repaint();
                    }
                }

                crate::infrastructure::io_priority::reset_thread_priority();
            }
        })
        .expect("failed to spawn folder-size-batch worker");

    (batch_tx, res_rx, cancel, generation)
}

/// Check if a path resides on an NTFS filesystem.
/// Uses `GetVolumeInformationW` with the drive root.
fn is_ntfs_volume(path: &std::path::Path) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

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

    String::from_utf16_lossy(&fs_name)
        .trim_end_matches('\0')
        .eq_ignore_ascii_case("NTFS")
}
