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
use eframe::egui;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_NOSOCKET};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

mod request_processing;

/// Maximum concurrent decode operations (RAM limiter)
/// Reduced from 5 to 3 to prevent RAM spikes on HDD folders
/// Each decode can use ~50-100MB for large images
const MAX_CONCURRENT_DECODES: usize = 3;

/// Semaphore to limit concurrent resource usage
pub struct Semaphore {
    count: Mutex<usize>,
    condvar: Condvar,
    max: usize,
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
    // Semaphore for RAM limiter
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DECODES));

    // 4 worker threads
    for _ in 0..4 {
        let queue = queue.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        let disk_cache = disk_cache.clone();
        let semaphore = semaphore.clone();
        let pending_deletions = pending_deletions.clone();

        std::thread::spawn(move || {
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
    // This applies to all 4 thumbnail worker threads
    io_priority::set_thread_priority(IOPriority::Background);

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

