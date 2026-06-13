use crate::app::folder_size_state::{FolderContentSummary, FolderSizeMessage};
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const NTFS_FOLDER_SIZE_SERVICE_RETRY_ATTEMPTS: usize = 20;
const NTFS_FOLDER_SIZE_SERVICE_INITIAL_RETRY_DELAY: Duration = Duration::from_millis(50);
const NTFS_FOLDER_SIZE_SERVICE_MAX_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Overall deadline for the retry loop to prevent blocking for minutes
/// on persistent transient failures (20 attempts × 8s timeout = ~160s worst case).
const NTFS_FOLDER_SIZE_SERVICE_DEADLINE_SECS: u64 = 30;

/// Payload for the disk-cache invalidation channel.
/// When `force` is `true` the existence guard is skipped — used for
/// app-initiated deletes where the Shell hasn't finished yet.
pub struct CacheInvalidationEntry {
    pub path: PathBuf,
    pub force: bool,
    /// If `Some`, this is a rename operation: the disk-cache row for `path`
    /// should be moved to `rename_to` rather than deleted.
    pub rename_to: Option<PathBuf>,
}

fn should_skip_exists_guard(path: &std::path::Path) -> bool {
    crate::infrastructure::onedrive::is_cloud_sync_path(path)
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
) -> Result<(u64, u64, u64), String> {
    let mut last_error = String::from("Search service not available");
    let deadline = Instant::now() + Duration::from_secs(NTFS_FOLDER_SIZE_SERVICE_DEADLINE_SECS);
    let mut retry_delay = NTFS_FOLDER_SIZE_SERVICE_INITIAL_RETRY_DELAY;

    for attempt in 0..NTFS_FOLDER_SIZE_SERVICE_RETRY_ATTEMPTS {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".to_string());
        }
        if Instant::now() >= deadline {
            break;
        }

        match crate::infrastructure::global_search::folder_size(folder_path) {
            Ok(result) => return Ok(result),
            Err(error) => {
                let retryable = is_retryable_folder_size_service_error(&error);
                last_error = error;
                if !retryable || attempt + 1 >= NTFS_FOLDER_SIZE_SERVICE_RETRY_ATTEMPTS {
                    break;
                }

                // Sleep in small steps to remain responsive to cancel.
                let sleep_end = Instant::now() + retry_delay;
                while Instant::now() < sleep_end {
                    if cancel.load(Ordering::Acquire) {
                        return Err("cancelled".to_string());
                    }
                    let remaining = sleep_end.saturating_duration_since(Instant::now());
                    std::thread::sleep(remaining.min(Duration::from_millis(50)));
                }
                retry_delay = retry_delay
                    .checked_mul(2)
                    .unwrap_or(NTFS_FOLDER_SIZE_SERVICE_MAX_RETRY_DELAY)
                    .min(NTFS_FOLDER_SIZE_SERVICE_MAX_RETRY_DELAY);
            }
        }
    }

    Err(last_error)
}

fn is_cloud_sync_folder_size_service_lookup_error(
    folder_path: &std::path::Path,
    message: &str,
) -> bool {
    if !crate::infrastructure::onedrive::is_cloud_sync_path(folder_path) {
        return false;
    }

    let lower = message.to_ascii_lowercase();
    lower.contains("path not found in index") || lower.contains("invalid path")
}

fn query_cloud_sync_folder_size_with_timeout(
    folder_path: &std::path::Path,
    cancel: &Arc<AtomicBool>,
) -> Result<crate::infrastructure::windows::folder_size::FolderScanResult, String> {
    fn walk_directory(
        path: &std::path::Path,
        cancel: &Arc<AtomicBool>,
        depth: usize,
    ) -> Result<crate::infrastructure::windows::folder_size::FolderScanResult, String> {
        if cancel.load(Ordering::Acquire) {
            return Err("cancelled".to_string());
        }
        if depth > 256 {
            return Err("cloud-sync folder-size recursion depth exceeded".to_string());
        }

        match crate::infrastructure::onedrive::onedrive_read_directory(path) {
            crate::infrastructure::onedrive::IoTimeoutResult::Ok(entries) => {
                let mut totals =
                    crate::infrastructure::windows::folder_size::FolderScanResult::default();
                for (name, attrs, size, _modified) in entries {
                    if cancel.load(Ordering::Acquire) {
                        return Err("cancelled".to_string());
                    }

                    let child_path = path.join(name);
                    let is_dir = (attrs & 0x10) != 0;
                    if is_dir {
                        totals.folder_count = totals.folder_count.saturating_add(1);
                        let child_totals = walk_directory(&child_path, cancel, depth + 1)?;
                        totals.total_size =
                            totals.total_size.saturating_add(child_totals.total_size);
                        totals.file_count =
                            totals.file_count.saturating_add(child_totals.file_count);
                        totals.folder_count = totals
                            .folder_count
                            .saturating_add(child_totals.folder_count);
                    } else {
                        totals.total_size = totals.total_size.saturating_add(size);
                        totals.file_count = totals.file_count.saturating_add(1);
                    }
                }
                Ok(totals)
            }
            crate::infrastructure::onedrive::IoTimeoutResult::Timeout => {
                Err("onedrive directory enumeration timeout".to_string())
            }
            crate::infrastructure::onedrive::IoTimeoutResult::Err(kind) => {
                Err(format!("onedrive directory enumeration failed: {:?}", kind))
            }
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
    if let Err(e) = std::thread::Builder::new()
        .name("disk-cache-invalidation".into())
        .spawn(move || {
            while let Ok(mut entries) = disk_cache_invalidation_rx.recv() {
                while let Ok(mut more) = disk_cache_invalidation_rx.try_recv() {
                    entries.append(&mut more);
                }

                let mut unique_paths = std::collections::HashSet::with_capacity(entries.len());
                let mut skipped_existing_files = 0usize;

                for entry in entries {
                    if !unique_paths.insert(entry.path.clone()) {
                        continue;
                    }

                    if let Some(new_path) = entry.rename_to {
                        // Rename: migrate the disk-cache row to the new path
                        // so thumbnails survive rename without re-extraction.
                        disk_cache_for_invalidation
                            .rename_cache_entry(&entry.path, &new_path);
                    } else if entry.force {
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
                        if !crate::infrastructure::onedrive::fast_is_dir(entry.path.as_path()) {
                            skipped_existing_files += 1;
                            continue;
                        }

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

                if skipped_existing_files > 0 {
                    log::debug!(
                        "[CACHE-INVALIDATION] Skipped {} existing file invalidation(s)",
                        skipped_existing_files
                    );
                }
            }
        })
    {
        log::error!("[CACHE-INVALIDATION] Failed to spawn worker thread: {e}. Cache invalidation disabled.");
    }
    disk_cache_invalidation_tx
}

pub(in crate::app) fn spawn_folder_preview_workers(
    ctx: &egui::Context,
    disk_cache: Arc<ThumbnailDiskCache>,
    folder_composer: Arc<crate::infrastructure::folder_compose::FolderComposer>,
    trace: Arc<crate::workers::folder_preview_worker::FolderPreviewTraceCounters>,
) -> (
    crossbeam_channel::Sender<crate::workers::folder_preview_worker::FolderPreviewRequest>,
    mpsc::Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,
) {
    // M-18: crossbeam Receiver is Clone + Send + Sync — workers share it directly
    // without the Arc<Mutex<>> serialisation bottleneck.
    // Bounded(60): inherent backpressure prevents unbounded growth during rapid
    // navigation — preview is best-effort so dropping overflow is safe.
    let (folder_preview_tx, folder_preview_rx) = crossbeam_channel::bounded::<
        crate::workers::folder_preview_worker::FolderPreviewRequest,
    >(60);
    let (folder_preview_res_tx, folder_preview_res_rx) = mpsc::channel();

    {
        use crate::workers::folder_preview_worker::spawn_folder_preview_worker;
        // Folder-preview composition is dominated by Shell COM calls + SQLite
        // cache hits; 2 workers fully cover scroll bursts while keeping the
        // committed stack footprint low (each OS thread commits ~1 MB by default).
        let worker_count: usize = 2;
        for _ in 0..worker_count {
            spawn_folder_preview_worker(
                folder_preview_rx.clone(),
                folder_preview_res_tx.clone(),
                ctx.clone(),
                disk_cache.clone(),
                folder_composer.clone(),
                trace.clone(),
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
                    Ok((total_size, file_count, folder_count)) => {
                        log::info!(
                            "[FOLDER-SIZE] IPC complete path={} total_gb={:.2} files={} folders={}",
                            folder_path.display(),
                            total_size as f64 / 1_073_741_824.0,
                            file_count,
                            folder_count,
                        );
                        let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                            folder_path: folder_path.clone(),
                            summary: FolderContentSummary::complete(
                                total_size,
                                file_count,
                                folder_count,
                            ),
                        });
                        folder_size_ctx.request_repaint();
                        continue;
                    }
                    Err(e) => {
                        if e == "cancelled" {
                            let _ = folder_size_res_tx
                                .send(FolderSizeMessage::Cancelled { folder_path });
                            folder_size_ctx.request_repaint();
                            continue;
                        }

                        if is_cloud_sync_folder_size_service_lookup_error(&folder_path, &e) {
                            match query_cloud_sync_folder_size_with_timeout(
                                &folder_path,
                                &folder_size_cancel_worker,
                            ) {
                                Ok(result) => {
                                    log::info!(
                                        "[FOLDER-SIZE] Cloud sync fallback complete path={} total_gb={:.2} files={} folders={}",
                                        folder_path.display(),
                                        result.total_size as f64 / 1_073_741_824.0,
                                        result.file_count,
                                        result.folder_count,
                                    );
                                    let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                                        folder_path: folder_path.clone(),
                                        summary: FolderContentSummary::complete(
                                            result.total_size,
                                            result.file_count,
                                            result.folder_count,
                                        ),
                                    });
                                    folder_size_ctx.request_repaint();
                                    continue;
                                }
                                Err(fallback_error) if fallback_error == "cancelled" => {
                                    let _ = folder_size_res_tx
                                        .send(FolderSizeMessage::Cancelled { folder_path });
                                    folder_size_ctx.request_repaint();
                                    continue;
                                }
                                Err(fallback_error) => {
                                    log::warn!(
                                        "[FOLDER-SIZE] Cloud sync fallback failed path={} reason={}",
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
                        let _ =
                            folder_size_res_tx.send(FolderSizeMessage::Cancelled { folder_path });
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
                            summary: FolderContentSummary::size_only(partial_size),
                        });
                        ctx_clone.request_repaint();
                    },
                );

            match result {
                Some(result) => {
                    log::info!(
                        "[FOLDER-SIZE] Fallback complete path={} total_gb={:.2} files={} folders={}",
                        folder_path.display(),
                        result.total_size as f64 / 1_073_741_824.0,
                        result.file_count,
                        result.folder_count,
                    );
                    let _ = folder_size_res_tx.send(FolderSizeMessage::Complete {
                        folder_path,
                        summary: FolderContentSummary::complete(
                            result.total_size,
                            result.file_count,
                            result.folder_count,
                        ),
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

    if let Err(e) = std::thread::Builder::new()
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
                        Ok((total_size, _file_count, _folder_count)) => {
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
                            if is_cloud_sync_folder_size_service_lookup_error(&path, &error) {
                                match query_cloud_sync_folder_size_with_timeout(&path, &cancel_worker)
                                {
                                    Ok(result) => {
                                        let _ = res_tx.send(BatchSizeResult {
                                            folder_path: path,
                                            total_size: Some(result.total_size),
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
                                            "[FOLDER-SIZE] Batch cloud sync fallback failed path={} reason={}",
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
                    if let Some(result) = result {
                        let _ = res_tx.send(BatchSizeResult {
                            folder_path: path,
                            total_size: Some(result.total_size),
                            request_epoch: req_epoch,
                        });
                        ctx.request_repaint();
                    }
                }

                crate::infrastructure::io_priority::reset_thread_priority();
            }
        })
    {
        log::error!("[FOLDER-SIZE] Failed to spawn batch worker thread: {e}. Folder size calculation disabled.");
    }

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
