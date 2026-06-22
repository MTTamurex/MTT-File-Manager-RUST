//! Thumbnail worker thread management
//!
//! Spawns worker threads and manages the thumbnail extraction lifecycle.
//!
//! PERFORMANCE CRITICAL: Uses timeout-protected I/O for OneDrive files to prevent
//! worker thread blocking on cloud-only files.

use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::thumbnail::queue::PriorityThumbnailQueue;
use crate::workers::thumbnail::types::ThumbnailRequestSource;
use crate::workers::thumbnail::SharedBulkThumbnailProgress;
use crossbeam_channel::Sender;
use eframe::egui;
use parking_lot::{Condvar, Mutex};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_NOSOCKET};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

mod request_processing;

/// Hard RAM safety cap for concurrent decode operations.
/// Each decode can temporarily use tens of MB, so keep bounded even on high-core CPUs.
const MAX_CONCURRENT_DECODES_HARD_CAP: usize = 4;
const CACHE_WRITE_QUEUE_CAP: usize = 1024;

pub(super) struct ThumbnailCacheWriteRequest {
    path: PathBuf,
    modified: SystemTime,
    requested_size: u32,
    data: Vec<u8>,
    width: u32,
    height: u32,
}

/// Semaphore to limit concurrent resource usage
pub struct Semaphore {
    count: Mutex<usize>,
    condvar: Condvar,
    max: usize,
}

struct SemaphorePermit<'a> {
    semaphore: &'a Semaphore,
}

impl Semaphore {
    fn new(max: usize) -> Self {
        Self {
            count: Mutex::new(0),
            condvar: Condvar::new(),
            max,
        }
    }

    fn acquire(&self) {
        let mut count = self.count.lock();
        while *count >= self.max {
            self.condvar.wait(&mut count);
        }
        *count += 1;
    }

    fn release(&self) {
        {
            let mut count = self.count.lock();
            if *count > 0 {
                *count -= 1;
            }
        }
        self.condvar.notify_one();
    }

    fn acquire_guard(&self) -> SemaphorePermit<'_> {
        self.acquire();
        SemaphorePermit { semaphore: self }
    }
}

impl Drop for SemaphorePermit<'_> {
    fn drop(&mut self) {
        self.semaphore.release();
    }
}

fn available_cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn compute_thumbnail_worker_count(cpu_count: usize) -> usize {
    // Decode parallelism is bounded by `compute_decode_limit` (max 4) via the
    // shared semaphore, so worker counts above 4 only add idle threads waiting
    // on the semaphore — each costing ~1 MB of committed stack RAM by default.
    // Cap at 4 and request smaller stacks at spawn time to keep the baseline tight.
    cpu_count.clamp(1, 4)
}

fn compute_decode_limit(worker_count: usize) -> usize {
    // Decode parallelism is already bounded by MAX_CONCURRENT_DECODES_HARD_CAP.
    // The old thresholds never exceeded 2 because worker_count is capped at 4,
    // which made slow video folders advance in visible 2-item batches.
    worker_count.clamp(1, MAX_CONCURRENT_DECODES_HARD_CAP)
}

/// Spawns thumbnail worker threads with concurrency limiting
#[allow(clippy::too_many_arguments)]
pub fn spawn_thumbnail_workers(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
    pending_deletions: Arc<dashmap::DashMap<std::path::PathBuf, ()>>,
    bulk_thumbnail_progress: SharedBulkThumbnailProgress,
    bulk_thumbnail_completed: Arc<AtomicUsize>,
    bulk_thumbnail_session: Arc<AtomicU64>,
) {
    let cpu_count = available_cpu_count();
    let worker_count = compute_thumbnail_worker_count(cpu_count);
    let decode_limit = compute_decode_limit(worker_count);

    // Semaphore for RAM limiter
    let semaphore = Arc::new(Semaphore::new(decode_limit));

    // Virtual drives (Cryptomator/WinFsp/Dokany): limit to 1 concurrent extraction
    // to avoid overwhelming the FUSE driver, which crashes under sustained parallel I/O.
    let virtual_drive_semaphore = Arc::new(Semaphore::new(1));

    log::info!(
        "[THUMB-PIPELINE] workers={} decode_limit={} cpu_count={}",
        worker_count,
        decode_limit,
        cpu_count
    );

    let (cache_write_tx, cache_write_rx) =
        crossbeam_channel::bounded::<ThumbnailCacheWriteRequest>(CACHE_WRITE_QUEUE_CAP);
    {
        let disk_cache = disk_cache.clone();
        let spawn_result = std::thread::Builder::new()
            .name("thumb-cache-writer".to_string())
            .stack_size(256 * 1024)
            .spawn(move || thumbnail_cache_writer_loop(disk_cache, cache_write_rx));

        if let Err(e) = spawn_result {
            log::warn!("[THUMB-PIPELINE] Failed to spawn cache writer thread: {e}");
        }
    }

    // Adaptive worker count based on available CPU resources.
    for worker_id in 0..worker_count {
        let queue = queue.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        let disk_cache = disk_cache.clone();
        let semaphore = semaphore.clone();
        let virtual_drive_semaphore = virtual_drive_semaphore.clone();
        let pending_deletions = pending_deletions.clone();
        let bulk_thumbnail_progress = bulk_thumbnail_progress.clone();
        let bulk_thumbnail_completed = bulk_thumbnail_completed.clone();
        let bulk_thumbnail_session = bulk_thumbnail_session.clone();
        let cache_write_tx = cache_write_tx.clone();

        let spawn_result = std::thread::Builder::new()
            .name(format!("thumb-worker-{}", worker_id))
            .stack_size(512 * 1024)
            .spawn(move || {
                thumbnail_worker_loop(
                    queue,
                    tx,
                    ctx,
                    gen_tracker,
                    disk_cache,
                    semaphore,
                    virtual_drive_semaphore,
                    pending_deletions,
                    bulk_thumbnail_progress,
                    bulk_thumbnail_completed,
                    bulk_thumbnail_session,
                    cache_write_tx,
                );
            });

        if let Err(e) = spawn_result {
            log::warn!(
                "[THUMB-PIPELINE] Failed to spawn worker {}: {}",
                worker_id,
                e
            );
        }
    }

    // Spawn the deferred-retry thread.
    // It polls the unsafe-path registry every ~1 s and re-injects requests
    // into the queue once classify_file_read_safety returns Safe.
    // By the time it calls classify_file_read_safety, Phase 1 guarantees that
    // actively-written files are checked via std::fs::metadata only (share-all
    // flags), never via the write-lock probe.
    {
        let queue = queue.clone();
        let gen_tracker = gen_tracker.clone();

        let spawn_result = std::thread::Builder::new()
            .name("thumb-deferred-retry".to_string())
            .stack_size(256 * 1024)
            .spawn(move || {
                deferred_retry_loop(queue, gen_tracker);
            });

        if let Err(e) = spawn_result {
            log::warn!(
                "[THUMB-PIPELINE] Failed to spawn deferred-retry thread: {}",
                e
            );
        }
    }
}

fn thumbnail_cache_writer_loop(
    disk_cache: Arc<ThumbnailDiskCache>,
    cache_write_rx: crossbeam_channel::Receiver<ThumbnailCacheWriteRequest>,
) {
    let _priority_guard = io_priority::ThreadPriorityGuard::new(IOPriority::Background);

    while let Ok(request) = cache_write_rx.recv() {
        let cache_start = std::time::Instant::now();
        if let Err(e) = disk_cache.put(
            &request.path,
            request.modified,
            request.requested_size,
            &request.data,
            request.width,
            request.height,
        ) {
            log::error!(
                "[Thumbnail-CACHE] PUT FAILED for {:?}: {:?}",
                request.path.file_name(),
                e
            );
        }

        let cache_ms = cache_start.elapsed().as_millis();
        if cache_ms >= 25 {
            log::info!(
                "[THUMB-CACHE-WRITER] put={:.1}ms {:?} {}x{}",
                cache_ms as f64,
                request.path.file_name(),
                request.width,
                request.height
            );
        }
    }
}

/// Background thread that periodically retries thumbnail extraction for files that
/// were previously deferred because they were being written (e.g. active torrent
/// download with qBittorrent sparse pre-allocation).
///
/// Flow:
///   1. Sleep 1 s.
///   2. Drain the `UNSAFE_REGISTRY`.
///   3. For each entry, call `classify_file_read_safety` (cheap; no write-lock
///      probe for actively-writing files after Phase 1).
///   4. If `Safe` → clear the transient backoff and re-push into the queue.
///   5. If still unsafe AND not expired → re-insert into the registry.
///   6. If expired (>30 min) → drop silently.
///
/// A 4-permit semaphore limits concurrent re-classify probes to avoid an I/O
/// spike on folders with many partial files.
fn deferred_retry_loop(queue: Arc<PriorityThumbnailQueue>, gen_tracker: Arc<AtomicUsize>) {
    use crate::infrastructure::windows::file_flags::{classify_file_read_safety, FileReadSafety};
    use crate::workers::thumbnail::{
        clear_transient_failure, defer_unsafe_thumbnail, deferred_entry_expired,
        drain_unsafe_registry,
    };

    // 4-permit semaphore: cap concurrent classify probes to avoid I/O spikes.
    let probe_sem = Arc::new(Semaphore::new(4));

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));

        let entries = drain_unsafe_registry();
        if entries.is_empty() {
            continue;
        }

        let current_gen = gen_tracker.load(Ordering::Relaxed);

        for (path, entry) in entries {
            if entry.req_generation != current_gen {
                log::debug!(
                    "[THUMB-RETRY] Dropping stale deferred entry: {:?}",
                    path.file_name()
                );
                continue;
            }

            // Drop stale entries that have been waiting too long.
            if deferred_entry_expired(&entry) {
                log::debug!(
                    "[THUMB-RETRY] Dropping expired deferred entry: {:?}",
                    path.file_name()
                );
                continue;
            }

            let probe_sem = probe_sem.clone();
            let queue = queue.clone();

            let _permit = probe_sem.acquire_guard();
            let safety = classify_file_read_safety(&path);

            match safety {
                FileReadSafety::Safe => {
                    clear_transient_failure(&path);
                    // Re-queue at the original priority so the UI sees the thumbnail
                    // appear within one worker cycle (~tens of ms).
                    queue.push(
                        path.clone(),
                        current_gen,
                        entry.req_size,
                        entry.req_priority,
                        entry.req_modified,
                    );
                    log::debug!(
                        "[THUMB-RETRY] Re-queued after becoming safe: {:?}",
                        path.file_name()
                    );
                }
                _ => {
                    // Still not safe — re-insert into the registry for the next tick.
                    defer_unsafe_thumbnail(path, entry);
                }
            }
        }
    }
}

/// RAII guard for COM and Media Foundation initialization.
/// Ensures cleanup (CoUninitialize / MFShutdown) even if the thread panics.
struct ComMfGuard {
    com_initialized: bool,
    mf_initialized: bool,
}

impl Drop for ComMfGuard {
    fn drop(&mut self) {
        unsafe {
            if self.mf_initialized {
                let _ = MFShutdown();
            }
            if self.com_initialized {
                crate::workers::thumbnail::extraction::stage2_wic::drop_thread_local_factory();
                CoUninitialize();
            }
        }
    }
}

/// Main worker thread loop for thumbnail extraction
#[allow(clippy::too_many_arguments)]
fn thumbnail_worker_loop(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
    semaphore: Arc<Semaphore>,
    virtual_drive_semaphore: Arc<Semaphore>,
    pending_deletions: Arc<dashmap::DashMap<std::path::PathBuf, ()>>,
    bulk_thumbnail_progress: SharedBulkThumbnailProgress,
    bulk_thumbnail_completed: Arc<AtomicUsize>,
    bulk_thumbnail_session: Arc<AtomicU64>,
    cache_write_tx: Sender<ThumbnailCacheWriteRequest>,
) {
    let mut last_repaint = Instant::now();

    // RAII guards: guarantee COM and Media Foundation cleanup even on panic.
    let _com = {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        ComMfGuard {
            com_initialized: true,
            mf_initialized: false,
        }
    };
    let _mf = {
        let mf_ok = unsafe { MFStartup(0x00020070, MFSTARTUP_NOSOCKET).is_ok() };
        if !mf_ok {
            log::error!("[ThumbnailWorker] Failed to initialize Media Foundation");
        }
        ComMfGuard {
            com_initialized: false,
            mf_initialized: mf_ok,
        }
    };

    // PERFORMANCE: Set background priority to minimize HDD contention with video playback
    // This applies to all 4 thumbnail worker threads.
    // RAII guard ensures THREAD_MODE_BACKGROUND_END is called even on panic.
    let _priority_guard = io_priority::ThreadPriorityGuard::new(IOPriority::Background);

    while let Some((
        path,
        req_gen,
        req_size,
        req_epoch,
        req_priority,
        req_modified,
        req_source,
        track_bulk_progress,
        req_bulk_session,
    )) = queue.pop()
    {
        let active_bulk_session = if track_bulk_progress {
            req_bulk_session
                .filter(|session| *session == bulk_thumbnail_session.load(Ordering::Relaxed))
        } else {
            None
        };
        let participates_in_bulk_scan = active_bulk_session.is_some();

        if track_bulk_progress
            && !participates_in_bulk_scan
            && matches!(req_source, ThumbnailRequestSource::BulkScan)
        {
            continue;
        }

        // Check generation match. If stale, still notify the UI so any
        // caller-side loading_set marker for this path is cleared; otherwise a
        // request dropped during a dual-panel generation race can block retries
        // until a manual refresh.
        if !participates_in_bulk_scan && req_gen != gen_tracker.load(Ordering::Relaxed) {
            let _ = tx.send(ThumbnailData {
                path: path.clone(),
                image_data: std::sync::Arc::new(Vec::new()),
                width: 0,
                height: 0,
                generation: req_gen,
                request_epoch: req_epoch,
                priority: req_priority,
                not_found: false,
                premultiplied: false,
            });
            ctx.request_repaint();
            continue;
        }

        if participates_in_bulk_scan {
            crate::workers::thumbnail::set_bulk_thumbnail_current_file(
                &bulk_thumbnail_progress,
                &path,
                active_bulk_session.unwrap(),
            );
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let is_virtual_bulk_scan = matches!(req_source, ThumbnailRequestSource::BulkScan)
                && io_priority::is_virtual_drive_path(&path);
            let _virtual_drive_permit = if is_virtual_bulk_scan {
                Some(virtual_drive_semaphore.acquire_guard())
            } else {
                None
            };

            request_processing::process_thumbnail_request(
                &path,
                req_gen,
                req_size,
                req_epoch,
                req_priority,
                req_modified,
                &tx,
                &ctx,
                &disk_cache,
                &semaphore,
                &pending_deletions,
                &mut last_repaint,
                &cache_write_tx,
            );

            if is_virtual_bulk_scan {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }));

        if let Err(e) = result {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown".to_string()
            };
            log::error!(
                "[ThumbnailWorker] panic processing {}: {}",
                path.display(),
                msg
            );
        }

        if participates_in_bulk_scan
            && active_bulk_session.unwrap() == bulk_thumbnail_session.load(Ordering::Relaxed)
        {
            bulk_thumbnail_completed.fetch_add(1, Ordering::Relaxed);
            ctx.request_repaint();
        }
    }
    // _mf and _com dropped here — MFShutdown() then CoUninitialize() guaranteed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_compute_thumbnail_worker_count_scales_down_and_caps() {
        assert_eq!(compute_thumbnail_worker_count(1), 1);
        assert_eq!(compute_thumbnail_worker_count(2), 2);
        assert_eq!(compute_thumbnail_worker_count(3), 3);
        assert_eq!(compute_thumbnail_worker_count(4), 4);
        assert_eq!(compute_thumbnail_worker_count(16), 4);
    }

    #[test]
    fn test_compute_decode_limit_tracks_worker_count_up_to_hard_cap() {
        assert_eq!(compute_decode_limit(1), 1);
        assert_eq!(compute_decode_limit(2), 2);
        assert_eq!(compute_decode_limit(3), 3);
        assert_eq!(compute_decode_limit(4), 4);
        assert_eq!(compute_decode_limit(16), MAX_CONCURRENT_DECODES_HARD_CAP);
    }

    #[test]
    fn test_semaphore_concurrency() {
        let max_concurrent = 2;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        let active_count = Arc::new(Mutex::new(0));

        let mut handles = vec![];

        for i in 0..5 {
            let semaphore = semaphore.clone();
            let active_count = active_count.clone();

            handles.push(thread::spawn(move || {
                semaphore.acquire();

                {
                    let mut count = active_count.lock();
                    *count += 1;
                    assert!(*count <= max_concurrent, "Too many threads!");
                    println!("Thread {} running. Active: {}", i, *count);
                }

                thread::sleep(Duration::from_millis(50));

                {
                    let mut count = active_count.lock();
                    *count -= 1;
                }

                semaphore.release();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }
}
