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
use crossbeam_channel::Sender;
use eframe::egui;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_NOSOCKET};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

mod request_processing;

/// Hard RAM safety cap for concurrent decode operations.
/// Each decode can temporarily use tens of MB, so keep bounded even on high-core CPUs.
const MAX_CONCURRENT_DECODES_HARD_CAP: usize = 4;

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
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count >= self.max {
            count = self.condvar.wait(count).unwrap_or_else(|e| e.into_inner());
        }
        *count += 1;
    }

    fn release(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        if *count > 0 {
            *count -= 1;
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
    // Keep a balanced default: enough workers to saturate I/O/decode without overscheduling.
    cpu_count.clamp(4, 8)
}

fn compute_decode_limit(worker_count: usize) -> usize {
    // Decode parallelism must stay tighter than worker count to cap peak RAM use.
    let limit = if worker_count >= 7 {
        4
    } else if worker_count >= 5 {
        3
    } else {
        2
    };
    limit.clamp(2, MAX_CONCURRENT_DECODES_HARD_CAP)
}

/// Spawns thumbnail worker threads with concurrency limiting
pub fn spawn_thumbnail_workers(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
    pending_deletions: Arc<dashmap::DashMap<std::path::PathBuf, ()>>,
) {
    let cpu_count = available_cpu_count();
    let worker_count = compute_thumbnail_worker_count(cpu_count);
    let decode_limit = compute_decode_limit(worker_count);

    // Semaphore for RAM limiter
    let semaphore = Arc::new(Semaphore::new(decode_limit));

    log::info!(
        "[THUMB-PIPELINE] workers={} decode_limit={} cpu_count={}",
        worker_count,
        decode_limit,
        cpu_count
    );

    // Adaptive worker count based on available CPU resources.
    for worker_id in 0..worker_count {
        let queue = queue.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        let disk_cache = disk_cache.clone();
        let semaphore = semaphore.clone();
        let pending_deletions = pending_deletions.clone();

        let spawn_result = std::thread::Builder::new()
            .name(format!("thumb-worker-{}", worker_id))
            .spawn(move || {
                thumbnail_worker_loop(
                    queue,
                    tx,
                    ctx,
                    gen_tracker,
                    disk_cache,
                    semaphore,
                    pending_deletions,
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
                CoUninitialize();
            }
        }
    }
}

/// Main worker thread loop for thumbnail extraction
fn thumbnail_worker_loop(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
    semaphore: Arc<Semaphore>,
    pending_deletions: Arc<dashmap::DashMap<std::path::PathBuf, ()>>,
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

    while let Some((path, req_gen, req_size, req_priority, req_modified)) = queue.pop() {
        // Check generation match - skip stale requests
        if req_gen != gen_tracker.load(Ordering::Relaxed) {
            continue;
        }

        request_processing::process_thumbnail_request(
            &path,
            req_gen,
            req_size,
            req_priority,
            req_modified,
            &tx,
            &ctx,
            &disk_cache,
            &semaphore,
            &pending_deletions,
            &mut last_repaint,
        );
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
                    let mut count = active_count.lock().unwrap();
                    *count += 1;
                    assert!(*count <= max_concurrent, "Too many threads!");
                    println!("Thread {} running. Active: {}", i, *count);
                }

                thread::sleep(Duration::from_millis(50));

                {
                    let mut count = active_count.lock().unwrap();
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

