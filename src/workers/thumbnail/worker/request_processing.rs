use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::diagnostic_logger::{diag_info, diag_warn, field_label, field_u64};
use crate::infrastructure::disk_cache::{ThumbnailCacheEntry, ThumbnailDiskCache};
use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::onedrive::{self, IoTimeoutResult};
use crate::infrastructure::windows::is_mpeg_ts_file;
use crate::workers::thumbnail::extraction::{
    generate_thumbnail_hybrid_detailed_with_target, ThumbnailExtractionOutcome,
};
use crate::workers::thumbnail::processing::resize::{get_bucket_size, resize_to_bucket};
use crossbeam_channel::Sender;
use eframe::egui;
use image::ImageFormat;
use std::time::{Instant, SystemTime};

use super::Semaphore;

fn try_decode_latest_cache_entry(
    disk_cache: &ThumbnailDiskCache,
    path: &std::path::Path,
    req_modified: u64,
    req_size: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let entry = disk_cache.get_latest(path)?;
    let w = entry.width;
    let h = entry.height;
    let rs = entry.requested_size;
    let cached_mod = entry.modified_at;

    let mtime_mismatch = req_modified > 0 && cached_mod > 0 && req_modified != cached_mod;

    if mtime_mismatch {
        log::debug!(
            "[Thumbnail-CACHE] LATEST match REJECTED (mtime mismatch): path={:?}, cached_mod={}, req_mod={}",
            path.file_name(), cached_mod, req_modified
        );
        return None;
    }

    if let Some(decoded) = decode_cache_entry(entry, req_size) {
        return Some(decoded);
    }

    log::debug!(
        "[Thumbnail-CACHE] LATEST match found but decode_cache_entry rejected! path={:?}, cached={}x{}, requested_size_in_db={}, req_size={}",
        path.file_name(), w, h, rs, req_size
    );
    None
}

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
        clear_failure_cache, clear_transient_failure, defer_unsafe_thumbnail, is_known_failure,
        is_permanent_failure, mark_as_failed, mark_as_temporarily_blocked,
        mark_as_transient_failure, DeferredThumbnailEntry,
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
            send_thumbnail_result(
                tx,
                req_priority,
                ThumbnailData {
                    path: path.clone(),
                    image_data: Vec::new(),
                    width: 0,
                    height: 0,
                    generation: req_gen,
                    priority: req_priority,
                    not_found: true,
                },
            );
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
        let can_retry_now =
            onedrive::is_onedrive_path(path) && onedrive::is_locally_available_safe(path);

        if can_retry_now {
            clear_failure_cache(path);
        } else {
            send_thumbnail_result(
                tx,
                req_priority,
                ThumbnailData {
                    path: path.clone(),
                    image_data: Vec::new(),
                    width: 0,
                    height: 0,
                    generation: req_gen,
                    priority: req_priority,
                    not_found: is_permanent_failure(path),
                },
            );
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
    // SAFETY: reject the cached entry when our request carries a valid
    // modified-time that differs from the DB row.  This prevents showing
    // a stale thumbnail from a *different* file that previously lived at
    // the same path (e.g., delete A, rename B → A).
    if final_result.is_none() {
        if let Some(decoded) =
            try_decode_latest_cache_entry(disk_cache, path, req_modified, req_size)
        {
            final_result = Some(decoded);
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
        send_thumbnail_result(
            tx,
            req_priority,
            ThumbnailData {
                path: path.clone(),
                image_data: data,
                width: w,
                height: h,
                generation: req_gen,
                priority: req_priority,
                not_found: false,
            },
        );
        throttle_repaint_with_priority(ctx, last_repaint, req_priority);
        return;
    }

    // --- CACHE MISS PATH: now we need to access the source file ---

    // EARLY EXIT 2: skip files that no longer exist.
    if onedrive::is_onedrive_path(path) {
        match onedrive::onedrive_exists(path) {
            IoTimeoutResult::Ok(false) => {
                mark_as_failed(path.clone());
                send_thumbnail_result(
                    tx,
                    req_priority,
                    ThumbnailData {
                        path: path.clone(),
                        image_data: Vec::new(),
                        width: 0,
                        height: 0,
                        generation: req_gen,
                        priority: req_priority,
                        not_found: true,
                    },
                );
                throttle_repaint_with_priority(ctx, last_repaint, req_priority);
                return;
            }
            IoTimeoutResult::Timeout => {
                mark_as_transient_failure(path.clone());
                log::warn!("[THUMB WORKER] exists() timeout during OneDrive availability check");
                diag_warn(
                    "thumbnail_worker",
                    "exists_timeout",
                    &[field_label("provider", "onedrive")],
                );
            }
            IoTimeoutResult::Ok(true) => {}
            IoTimeoutResult::Err(_) => {
                mark_as_transient_failure(path.clone());
                send_thumbnail_result(
                    tx,
                    req_priority,
                    ThumbnailData {
                        path: path.clone(),
                        image_data: Vec::new(),
                        width: 0,
                        height: 0,
                        generation: req_gen,
                        priority: req_priority,
                        not_found: false,
                    },
                );
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
            send_thumbnail_result(
                tx,
                req_priority,
                ThumbnailData {
                    path: path.clone(),
                    image_data: Vec::new(),
                    width: 0,
                    height: 0,
                    generation: req_gen,
                    priority: req_priority,
                    not_found: true,
                },
            );
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
                log::warn!("[THUMB WORKER] metadata() timeout during OneDrive metadata lookup");
                diag_warn(
                    "thumbnail_worker",
                    "metadata_timeout",
                    &[field_label("provider", "onedrive")],
                );
                SystemTime::UNIX_EPOCH
            }
            IoTimeoutResult::Err(_) => {
                mark_as_transient_failure(path.clone());
                SystemTime::UNIX_EPOCH
            }
        }
    };

    // STEP 0+1: acquire semaphore, then check cache + extract under concurrency limit.
    // This prevents N simultaneous WebP decodes (~4 MB each) from spiking RAM when
    // all workers find cache hits at the same time.
    if final_result.is_none() {
        // Cancellation: skip extraction if file is pending deletion.
        if pending_deletions.contains_key(path) {
            send_thumbnail_result(
                tx,
                req_priority,
                ThumbnailData {
                    path: path.clone(),
                    image_data: Vec::new(),
                    width: 0,
                    height: 0,
                    generation: req_gen,
                    priority: req_priority,
                    not_found: false,
                },
            );
            throttle_repaint_with_priority(ctx, last_repaint, req_priority);
            return;
        }

        // Wait until a slot is available.
        let _permit = semaphore.acquire_guard();

        // Re-check disk cache under semaphore (another worker may have filled it
        // while we waited, avoiding redundant extraction).
        if let Some(entry) = disk_cache.get(path, modified) {
            final_result = decode_cache_entry(entry, req_size);
        } else if let Some(decoded) =
            try_decode_latest_cache_entry(disk_cache, path, req_modified, req_size)
        {
            final_result = Some(decoded);
        }

        if final_result.is_none() {
            let bucket_size = get_bucket_size(req_size);
            let extract_start = std::time::Instant::now();
            match generate_thumbnail_hybrid_detailed_with_target(
                path,
                req_priority,
                pending_deletions,
                Some(bucket_size),
            ) {
                ThumbnailExtractionOutcome::Success((raw_data, w, h)) => {
                    let extract_ms = extract_start.elapsed().as_millis();
                    // Resize to bucket (frees RAM and optimizes GPU upload).
                    let resize_start = std::time::Instant::now();
                    let resized = resize_to_bucket(raw_data, w, h, bucket_size);
                    let resize_ms = resize_start.elapsed().as_millis();

                    // Save optimized version to SQLite.
                    let cache_start = std::time::Instant::now();
                    if let Err(e) =
                        disk_cache.put(path, modified, req_size, &resized.0, resized.1, resized.2)
                    {
                        log::error!(
                            "[Thumbnail-CACHE] PUT FAILED for {:?}: {:?}",
                            path.file_name(),
                            e
                        );
                    }
                    let cache_ms = cache_start.elapsed().as_millis();

                    if extract_ms >= 25 || cache_ms >= 10 {
                        log::info!(
                            "[THUMB-PERF] extract={:.1}ms resize={:.1}ms cache={:.1}ms total={:.1}ms {:?} {}x{}→{}x{} bucket={}",
                            extract_ms as f64,
                            resize_ms as f64,
                            cache_ms as f64,
                            (extract_ms + resize_ms + cache_ms) as f64,
                            path.file_name(),
                            w, h, resized.1, resized.2, bucket_size
                        );
                    }

                    diag_info(
                        "thumbnail_extraction",
                        "success",
                        &[
                            field_u64("result_w", resized.1 as u64),
                            field_u64("result_h", resized.2 as u64),
                            field_u64("req_bucket", bucket_size as u64),
                        ],
                    );

                    final_result = Some(resized);
                    clear_transient_failure(path);
                }
                ThumbnailExtractionOutcome::UnsafeToRead(reason) => {
                    // Active writes/downloads are transient by nature; do not
                    // escalate to permanent failure due to repeated retries.
                    log::debug!(
                        "[THUMB WORKER] Deferring thumbnail for {:?} (unsafe-to-read: {:?})",
                        path.file_name(),
                        reason
                    );
                    mark_as_temporarily_blocked(path.clone());
                    // Register in the deferred-retry registry so the retry thread
                    // re-queues this request as soon as the file becomes safe,
                    // without requiring user interaction (scroll / F5).
                    defer_unsafe_thumbnail(
                        path.to_path_buf(),
                        DeferredThumbnailEntry {
                            req_size,
                            req_priority,
                            req_modified,
                            req_generation: req_gen,
                            inserted_at: std::time::Instant::now(),
                        },
                    );
                }
                ThumbnailExtractionOutcome::Failed => {
                    // All 5 extraction stages failed — the system likely lacks
                    // the required codec (e.g., HEVC/MKV without K-Lite).
                    // Mark as permanent failure immediately so neither the
                    // worker nor the UI waste cycles retrying on every folder
                    // visit.  The user can press F5 to retry after installing
                    // a codec pack.
                    mark_as_failed(path.clone());
                    diag_warn("thumbnail_extraction", "permanent_failure", &[]);
                }
            }
        }
    }

    let permanently_failed = final_result.is_none() && is_permanent_failure(path);
    let (data, w, h) = final_result.unwrap_or_else(|| (Vec::new(), 0, 0));

    send_thumbnail_result(
        tx,
        req_priority,
        ThumbnailData {
            path: path.clone(),
            image_data: data,
            width: w,
            height: h,
            generation: req_gen,
            priority: req_priority,
            not_found: permanently_failed,
        },
    );
    throttle_repaint_with_priority(ctx, last_repaint, req_priority);
}

fn send_thumbnail_result(tx: &Sender<ThumbnailData>, priority: IOPriority, data: ThumbnailData) {
    if matches!(priority, IOPriority::Interactive) {
        let _ = tx.send(data);
        return;
    }

    match tx.try_send(data) {
        Ok(()) => {}
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            // Under saturation, drop non-interactive results to protect UI latency.
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
    }
}

fn decode_cache_entry(entry: ThumbnailCacheEntry, req_size: u32) -> Option<(Vec<u8>, u32, u32)> {
    if !cache_entry_satisfies_request(&entry, req_size) {
        return None;
    }

    let img = image::load_from_memory_with_format(&entry.data, ImageFormat::WebP).ok()?;
    let rgba = img.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let bucket_size = get_bucket_size(req_size);
    Some(resize_to_bucket(rgba.to_vec(), width, height, bucket_size))
}

pub(super) fn cache_entry_satisfies_request(entry: &ThumbnailCacheEntry, req_size: u32) -> bool {
    entry.satisfies_request(req_size)
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
            modified_at: 0,
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
            modified_at: 0,
        };
        assert!(cache_entry_satisfies_request(&entry, 512));
    }
}
