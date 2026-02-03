//! Thumbnail worker thread management
//!
//! Spawns worker threads and manages the thumbnail extraction lifecycle.

use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::onedrive;
use crate::workers::thumbnail::extraction::generate_thumbnail_hybrid;
use crate::workers::thumbnail::processing::resize::{get_bucket_size, resize_to_bucket};
use crate::workers::thumbnail::queue::PriorityThumbnailQueue;
use eframe::egui;
use image::ImageFormat;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
use windows::Win32::Media::MediaFoundation::{MFStartup, MFShutdown, MFSTARTUP_NOSOCKET};

/// Maximum concurrent decode operations (RAM limiter)
const MAX_CONCURRENT_DECODES: usize = 5;

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
        let mut count = self.count.lock().unwrap();
        while *count >= self.max {
            count = self.condvar.wait(count).unwrap();
        }
        *count += 1;
    }

    fn release(&self) {
        let mut count = self.count.lock().unwrap();
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

        std::thread::spawn(move || {
            thumbnail_worker_loop(queue, tx, ctx, gen_tracker, disk_cache, semaphore);
        });
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
) {
    let mut last_repaint = Instant::now();
    
    unsafe {
        // SAFETY: Initializing COM with Multithreaded support for this worker thread.
        // It is paired with `CoUninitialize` at the end of the thread loop.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        // SAFETY: Initialize Media Foundation ONCE per thread working with video processing.
        // This avoids the expensive overhead of MFStartup/MFShutdown for every single file.
        // MF_VERSION = 0x00020070, MFSTARTUP_NOSOCKET = 0x1
        if let Err(e) = MFStartup(0x00020070, MFSTARTUP_NOSOCKET) {
            eprintln!(
                "[ThumbnailWorker] Failed to initialize Media Foundation: {:?}",
                e
            );
        }
    }

    // PERFORMANCE: Set background priority to minimize HDD contention with video playback
    // This applies to all 4 thumbnail worker threads
    io_priority::set_thread_priority(IOPriority::Background);

    while let Some((path, req_gen, req_size, req_priority, req_modified)) = queue.pop() {
        // Check generation match - skip stale requests
        if req_gen != gen_tracker.load(Ordering::Relaxed) {
            continue;
        }

        process_thumbnail_request(
            &path,
            req_gen,
            req_size,
            req_priority,
            req_modified,
            &tx,
            &ctx,
            &disk_cache,
            &semaphore,
            &mut last_repaint,
        );
    }

    unsafe {
        // SAFETY: Cleaning up COM for this thread before exit.
        let _ = MFShutdown();
        CoUninitialize();
    }
}

/// Process a single thumbnail request
fn process_thumbnail_request(
    path: &std::path::PathBuf,
    req_gen: usize,
    req_size: u32,
    req_priority: IOPriority,
    req_modified: u64,
    tx: &Sender<ThumbnailData>,
    ctx: &egui::Context,
    disk_cache: &ThumbnailDiskCache,
    semaphore: &Semaphore,
    last_repaint: &mut Instant,
) {
    use crate::workers::thumbnail::{is_known_failure, mark_as_failed};

    // EARLY EXIT 1: Skip files that already failed in this session
    // Prevents repeated slow retries on broken files (e.g., 0x8004B205)
    if is_known_failure(path) {
        let _ = tx.send(ThumbnailData {
            path: path.clone(),
            image_data: Vec::new(),
            width: 0,
            height: 0,
            generation: req_gen,
        });
        throttle_repaint_with_priority(ctx, last_repaint, req_priority);
        return;
    }

    // EARLY EXIT 2: Skip cloud-only OneDrive files (not downloaded)
    // Only check OneDrive attributes if the path is in a OneDrive folder
    if onedrive::is_onedrive_path(path) && !onedrive::is_locally_available(path) {
        mark_as_failed(path.clone());
        let _ = tx.send(ThumbnailData {
            path: path.clone(),
            image_data: Vec::new(),
            width: 0,
            height: 0,
            generation: req_gen,
        });
        throttle_repaint_with_priority(ctx, last_repaint, req_priority);
        return;
    }

    // PERFORMANCE: Use modification time from folder enumeration when available.
    // This avoids a costly std::fs::metadata() syscall per file on HDD.
    let modified = if req_modified > 0 {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(req_modified)
    } else {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    };

    let mut final_result = None;

    // STEP 0: Check Disk Cache with SIZE VALIDATION
    if let Some((cached_bytes, cached_w, cached_h)) = disk_cache.get(path, modified) {
        let cached_max_dim = cached_w.max(cached_h);

        // Only use cache if it meets or exceeds the requested size
        // OR if dimensions are unknown (0) from old cache entries - regenerate those
        if cached_max_dim >= req_size && cached_max_dim > 0 {
            // Cache is good enough (or better), use it
            if let Ok(img) = image::load_from_memory_with_format(
                &cached_bytes,
                ImageFormat::WebP,
            ) {
                let rgba = img.to_rgba8();
                final_result = Some((rgba.to_vec(), rgba.width(), rgba.height()));
            }
        }
        // If cached_max_dim < req_size or == 0, fall through to regeneration
    }

    // STEP 1: Se não está em cache, decodifica com limite de concorrência
    if final_result.is_none() {
        // Aguarda até ter um slot disponível (max 4 decodes simultâneos)
        semaphore.acquire();

        // HYBRID PIPELINE com resize imediato
        if let Some((raw_data, w, h)) = generate_thumbnail_hybrid(path, req_priority) {
            // STEP 2: Resize to bucket (libera RAM e otimiza upload GPU)
            let bucket_size = get_bucket_size(req_size);
            let resized = resize_to_bucket(raw_data, w, h, bucket_size);

            // STEP 3: Salva versão otimizada em SQLite
            let _ = disk_cache.put(path, modified, &resized.0, resized.1, resized.2);

            // STEP 4: Usa a versão resizada (já otimizada)
            final_result = Some(resized);
        } else {
            // EXTRACTION FAILED: Mark as failed to skip future attempts
            mark_as_failed(path.clone());
        }
        // raw_data é dropado aqui automaticamente (libera RAM)

        // Libera slot
        semaphore.release();
    }

    let (data, w, h) = final_result.unwrap_or_else(|| (Vec::new(), 0, 0));

    let _ = tx.send(ThumbnailData {
        path: path.clone(),
        image_data: data,
        width: w,
        height: h,
        generation: req_gen,
    });
    throttle_repaint_with_priority(ctx, last_repaint, req_priority);
}

/// PERFORMANCE: Adaptive repaint throttling based on priority
/// Interactive requests get faster repaint (16ms ~ 60 FPS)
/// Background/Prefetch use slower repaint (33ms ~ 30 FPS) to reduce CPU load
fn throttle_repaint_with_priority(
    ctx: &egui::Context,
    last_repaint: &mut Instant,
    priority: IOPriority,
) {
    const INTERACTIVE_INTERVAL_MS: u64 = 16; // ~60 FPS for interactive
    const BACKGROUND_INTERVAL_MS: u64 = 33; // ~30 FPS for background

    let interval_ms = match priority {
        IOPriority::Interactive => INTERACTIVE_INTERVAL_MS,
        _ => BACKGROUND_INTERVAL_MS,
    };

    let elapsed = last_repaint.elapsed().as_millis() as u64;

    if elapsed >= interval_ms {
        ctx.request_repaint();
        *last_repaint = Instant::now();
    } else {
        // Schedule repaint for when throttle expires
        let remaining = interval_ms.saturating_sub(elapsed);
        ctx.request_repaint_after(std::time::Duration::from_millis(remaining));
    }
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