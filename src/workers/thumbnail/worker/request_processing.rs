use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::{ThumbnailCacheEntry, ThumbnailDiskCache};
use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::onedrive::{self, IoTimeoutResult};
use crate::infrastructure::windows::is_mpeg_ts_file;
use crate::workers::thumbnail::extraction::generate_thumbnail_hybrid;
use crate::workers::thumbnail::processing::resize::{get_bucket_size, resize_to_bucket};
use eframe::egui;
use image::ImageFormat;
use std::sync::mpsc::Sender;
use std::time::{Instant, SystemTime};

use super::Semaphore;

/// Process a single thumbnail request.
#[allow(clippy::too_many_arguments)]
pub(super) fn process_thumbnail_request(
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
    use crate::workers::thumbnail::{
        clear_failure_cache, clear_transient_failure, is_known_failure, is_permanent_failure,
        mark_as_failed, mark_as_transient_failure,
    };

    // Block .ts files that are NOT real MPEG-TS video (e.g. TypeScript sources).
    // Real MPEG-TS starts with sync byte 0x47; anything else is rejected permanently.
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ts"))
    {
        if !is_mpeg_ts_file(path) {
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

    // EARLY EXIT 1: Skip files that already failed in this session.
    // Prevents repeated slow retries on broken files (e.g., 0x8004B205).
    //
    // OneDrive special-case:
    // If a file was previously cloud-only (transient failure) but is now locally available,
    // clear backoff immediately and retry in the same request so thumbnails recover without
    // requiring manual refresh.
    if is_known_failure(path) {
        let can_retry_now = onedrive::is_onedrive_path(path) && onedrive::is_locally_available_safe(path);

        if can_retry_now {
            clear_failure_cache(path);
        } else {
            let _ = tx.send(ThumbnailData {
                path: path.clone(),
                image_data: Vec::new(),
                width: 0,
                height: 0,
                generation: req_gen,
                not_found: is_permanent_failure(path),
            });
            throttle_repaint_with_priority(ctx, last_repaint, req_priority);
            return;
        }
    }

    // PERFORMANCE FIX: Check SQLite disk cache BEFORE any source file I/O.
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
                log::debug!(
                    "[Thumbnail-CACHE] EXACT match found but decode_cache_entry rejected! path={:?}, cached={}x{}, requested_size_in_db={}, req_size={}",
                    path.file_name(), w, h, rs, req_size
                );
            }
        }
    }

    // Fallback: try get_latest (ignores mtime) when exact match missed.
    if final_result.is_none() {
        if let Some(entry) = disk_cache.get_latest(path) {
            let w = entry.width;
            let h = entry.height;
            let rs = entry.requested_size;
            if let Some(decoded) = decode_cache_entry(entry, req_size) {
                final_result = Some(decoded);
            } else {
                log::debug!(
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

    // If DB cache hit, send result and return - no source drive I/O needed.
    if let Some((data, w, h)) = final_result {
        clear_failure_cache(path);
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

    // --- CACHE MISS PATH: now we need to access the source file ---

    // EARLY EXIT 2: skip files that no longer exist.
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
                mark_as_transient_failure(path.clone());
                log::warn!("[THUMB WORKER] exists() timeout for {:?}", path);
            }
            IoTimeoutResult::Ok(true) => {}
            IoTimeoutResult::Err(_) => {
                mark_as_transient_failure(path.clone());
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
        }

        // NOTE: Do NOT skip cloud-only OneDrive files here.
        // Windows Explorer can still obtain thumbnails for placeholders via Shell/
        // thumbnail cache providers. We should attempt extraction and let the pipeline decide.
    } else {
        // Non-OneDrive: use fast_path_exists (GetFileAttributesW).
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

    // Use modification time from folder enumeration when available.
    let modified = if req_modified > 0 {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(req_modified)
    } else {
        // Timeout-protected metadata for OneDrive.
        match onedrive::onedrive_metadata(path) {
            IoTimeoutResult::Ok(metadata) => metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            IoTimeoutResult::Timeout => {
                mark_as_transient_failure(path.clone());
                log::warn!("[THUMB WORKER] metadata() timeout for {:?}", path);
                SystemTime::UNIX_EPOCH
            }
            IoTimeoutResult::Err(_) => {
                mark_as_transient_failure(path.clone());
                SystemTime::UNIX_EPOCH
            }
        }
    };

    // STEP 0: check disk cache with size validation (exact mtime, then fallback).
    if final_result.is_none() {
        if let Some(entry) = disk_cache.get(path, modified) {
            final_result = decode_cache_entry(entry, req_size);
        } else if let Some(entry) = disk_cache.get_latest(path) {
            final_result = decode_cache_entry(entry, req_size);
        }
    }

    // STEP 1: if not cached, decode with concurrency limit.
    if final_result.is_none() {
        // Cancellation: skip extraction if file is pending deletion.
        if pending_deletions.contains_key(path) {
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

        // Wait until a slot is available.
        semaphore.acquire();

        if let Some((raw_data, w, h)) =
            generate_thumbnail_hybrid(path, req_priority, pending_deletions)
        {
            // Resize to bucket (frees RAM and optimizes GPU upload).
            let bucket_size = get_bucket_size(req_size);
            let resized = resize_to_bucket(raw_data, w, h, bucket_size);

            // Save optimized version to SQLite.
            if let Err(e) = disk_cache.put(path, modified, req_size, &resized.0, resized.1, resized.2)
            {
                log::error!(
                    "[Thumbnail-CACHE] PUT FAILED for {:?}: {:?}",
                    path.file_name(),
                    e
                );
            }

            final_result = Some(resized);
            clear_transient_failure(path);
        } else {
            // Extraction failed: mark as failed to skip future attempts.
            mark_as_transient_failure(path.clone());
        }

        // Release slot.
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

pub(super) fn cache_entry_satisfies_request(entry: &ThumbnailCacheEntry, req_size: u32) -> bool {
    let cached_max_dim = entry.width.max(entry.height);
    if cached_max_dim == 0 {
        return false;
    }

    cached_max_dim >= req_size || entry.requested_size >= req_size
}

/// Adaptive repaint throttling based on priority.
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
        // Schedule repaint for when throttle expires.
        let remaining = interval_ms.saturating_sub(elapsed);
        ctx.request_repaint_after(std::time::Duration::from_millis(remaining));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
