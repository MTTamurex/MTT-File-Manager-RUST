//! Thumbnail worker thread management
//!
//! Spawns worker threads and manages the thumbnail extraction lifecycle.
//!
//! PERFORMANCE CRITICAL: Uses timeout-protected I/O for OneDrive files to prevent
//! worker thread blocking on cloud-only files.

use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::{ThumbnailCacheEntry, ThumbnailDiskCache};
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::onedrive::{self, IoTimeoutResult};
use crate::workers::thumbnail::extraction::generate_thumbnail_hybrid;
use crate::workers::thumbnail::processing::resize::{get_bucket_size, resize_to_bucket};
use crate::workers::thumbnail::queue::PriorityThumbnailQueue;
use eframe::egui;
use image::ImageFormat;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};
use windows::Win32::Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_NOSOCKET};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

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
            &pending_deletions,
            &mut last_repaint,
        );
    }
    // _mf and _com dropped here — MFShutdown() then CoUninitialize() guaranteed
}

/// Process a single thumbnail request
#[allow(clippy::too_many_arguments)]
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
    pending_deletions: &dashmap::DashMap<std::path::PathBuf, ()>,
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
            not_found: false,
        });
        throttle_repaint_with_priority(ctx, last_repaint, req_priority);
        return;
    }

    // PERFORMANCE FIX: Check SQLite disk cache BEFORE any source file I/O.
    // Previously, the worker called onedrive_exists() and onedrive_metadata()
    // on the source file BEFORE checking the DB — causing unnecessary disk I/O
    // on USB HDDs even when all thumbnails were already cached in SQLite on NVMe.
    //
    // When req_modified > 0 (common case — provided by FileEntry from directory
    // enumeration), we can check the DB immediately without touching the source drive.
    let mut final_result = None;

    if req_modified > 0 {
        let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(req_modified);
        if let Some(entry) = disk_cache.get(path, modified) {
            let w = entry.width;
            let h = entry.height;
            let rs = entry.requested_size;
            if let Some(decoded) = decode_cache_entry(entry, req_size) {
                final_result = Some(decoded);
            } else {
                log::warn!(
                    "[Thumbnail-CACHE] EXACT match found but decode_cache_entry rejected! path={:?}, cached={}x{}, requested_size_in_db={}, req_size={}",
                    path.file_name(), w, h, rs, req_size
                );
            }
        }
    }

    // Fallback: try get_latest (ignores mtime) when exact match missed.
    // This handles virtual drives that report unstable mtimes between remounts,
    // drives not auto-detected as virtual, and req_modified == 0 cases.
    // Cost: one extra SQLite query on the local NVMe — negligible.
    if final_result.is_none() {
        if let Some(entry) = disk_cache.get_latest(path) {
            let w = entry.width;
            let h = entry.height;
            let rs = entry.requested_size;
            if let Some(decoded) = decode_cache_entry(entry, req_size) {
                final_result = Some(decoded);
            } else {
                log::warn!(
                    "[Thumbnail-CACHE] LATEST match found but decode_cache_entry rejected! path={:?}, cached={}x{}, requested_size_in_db={}, req_size={}",
                    path.file_name(), w, h, rs, req_size
                );
            }
        } else {
            log::debug!(
                "[Thumbnail-CACHE] NO entry in DB at all for path={:?}, req_modified={}, req_size={}",
                path.file_name(), req_modified, req_size
            );
        }
    }

    // If DB cache hit, send result and return — no source drive I/O needed
    if let Some((data, w, h)) = final_result {
        let _ = tx.send(ThumbnailData {
            path: path.clone(),
            image_data: data,
            width: w,
            height: h,
            generation: req_gen,
            not_found: false,
        });
        throttle_repaint_with_priority(ctx, last_repaint, req_priority);
        return;
    }

    // --- CACHE MISS PATH: Now we need to access the source file ---

    // EARLY EXIT 2: Skip files that no longer exist (e.g., stale folder covers)
    // CRITICAL: Use fast_path_exists for non-OneDrive (GetFileAttributesW, no file handle),
    // timeout-protected exists for OneDrive paths.
    if onedrive::is_onedrive_path(path) {
        match onedrive::onedrive_exists(path) {
            IoTimeoutResult::Ok(false) => {
                mark_as_failed(path.clone());
                let _ = tx.send(ThumbnailData {
                    path: path.clone(),
                    image_data: Vec::new(),
                    width: 0,
                    height: 0,
                    generation: req_gen,
                    not_found: true,
                });
                throttle_repaint_with_priority(ctx, last_repaint, req_priority);
                return;
            }
            IoTimeoutResult::Timeout => {
                log::warn!("[THUMB WORKER] exists() timeout for {:?}", path);
            }
            IoTimeoutResult::Ok(true) => {}
            IoTimeoutResult::Err(_) => {
                mark_as_failed(path.clone());
                throttle_repaint_with_priority(ctx, last_repaint, req_priority);
                return;
            }
        }

        // EARLY EXIT 3: Skip cloud-only OneDrive files (not downloaded)
        if !onedrive::is_locally_available_safe(path) {
            mark_as_failed(path.clone());
            let _ = tx.send(ThumbnailData {
                path: path.clone(),
                image_data: Vec::new(),
                width: 0,
                height: 0,
                generation: req_gen,
                not_found: false,
            });
            throttle_repaint_with_priority(ctx, last_repaint, req_priority);
            return;
        }
    } else {
        // Non-OneDrive: Use fast_path_exists (GetFileAttributesW — no file handle, no download)
        if !onedrive::fast_path_exists(path) {
            mark_as_failed(path.clone());
            let _ = tx.send(ThumbnailData {
                path: path.clone(),
                image_data: Vec::new(),
                width: 0,
                height: 0,
                generation: req_gen,
                not_found: true,
            });
            throttle_repaint_with_priority(ctx, last_repaint, req_priority);
            return;
        }
    }

    // PERFORMANCE: Use modification time from folder enumeration when available.
    // This avoids a costly std::fs::metadata() syscall per file on HDD.
    let modified = if req_modified > 0 {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(req_modified)
    } else {
        // CRITICAL FIX: Use timeout-protected metadata for OneDrive
        // std::fs::metadata() can block indefinitely on cloud-only files
        match onedrive::onedrive_metadata(path) {
            IoTimeoutResult::Ok(metadata) => metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            IoTimeoutResult::Timeout => {
                log::warn!("[THUMB WORKER] metadata() timeout for {:?}", path);
                SystemTime::UNIX_EPOCH
            }
            IoTimeoutResult::Err(_) => SystemTime::UNIX_EPOCH,
        }
    };

    // STEP 0: Check Disk Cache with size validation (exact mtime, then fallback)
    if final_result.is_none() {
        if let Some(entry) = disk_cache.get(path, modified) {
            final_result = decode_cache_entry(entry, req_size);
        } else if let Some(entry) = disk_cache.get_latest(path) {
            final_result = decode_cache_entry(entry, req_size);
        }
    }

    // STEP 1: Se não está em cache, decodifica com limite de concorrência
    if final_result.is_none() {
        // CANCELLATION: Skip extraction if file is pending deletion
        if pending_deletions.contains_key(path) {
            return;
        }

        // Aguarda até ter um slot disponível (max 4 decodes simultâneos)
        semaphore.acquire();

        // HYBRID PIPELINE com resize imediato
        if let Some((raw_data, w, h)) =
            generate_thumbnail_hybrid(path, req_priority, pending_deletions)
        {
            // STEP 2: Resize to bucket (libera RAM e otimiza upload GPU)
            let bucket_size = get_bucket_size(req_size);
            let resized = resize_to_bucket(raw_data, w, h, bucket_size);

            // STEP 3: Salva versão otimizada em SQLite
            if let Err(e) =
                disk_cache.put(path, modified, req_size, &resized.0, resized.1, resized.2)
            {
                log::error!(
                    "[Thumbnail-CACHE] PUT FAILED for {:?}: {:?}",
                    path.file_name(),
                    e
                );
            }

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
        not_found: false,
    });
    throttle_repaint_with_priority(ctx, last_repaint, req_priority);
}

fn decode_cache_entry(entry: ThumbnailCacheEntry, req_size: u32) -> Option<(Vec<u8>, u32, u32)> {
    if !cache_entry_satisfies_request(&entry, req_size) {
        return None;
    }

    let img = image::load_from_memory_with_format(&entry.data, ImageFormat::WebP).ok()?;
    let rgba = img.to_rgba8();
    Some((rgba.to_vec(), rgba.width(), rgba.height()))
}

fn cache_entry_satisfies_request(entry: &ThumbnailCacheEntry, req_size: u32) -> bool {
    let cached_max_dim = entry.width.max(entry.height);
    if cached_max_dim == 0 {
        return false;
    }

    cached_max_dim >= req_size || entry.requested_size >= req_size
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

    #[test]
    fn test_cache_entry_satisfies_request_with_sufficient_dimensions() {
        let entry = ThumbnailCacheEntry {
            data: Vec::new(),
            width: 512,
            height: 320,
            requested_size: 256,
        };
        assert!(cache_entry_satisfies_request(&entry, 512));
    }

    #[test]
    fn test_cache_entry_satisfies_request_with_requested_size_fallback() {
        let entry = ThumbnailCacheEntry {
            data: Vec::new(),
            width: 128,
            height: 128,
            requested_size: 512,
        };
        assert!(cache_entry_satisfies_request(&entry, 512));
    }
}
