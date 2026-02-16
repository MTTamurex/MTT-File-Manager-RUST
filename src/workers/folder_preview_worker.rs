//! Folder preview worker for async thumbnail extraction using Windows Shell API
//!
//! Uses IThumbnailCache with WTS_FORCEEXTRACTION to bypass Windows thumbnail cache
//! and avoid black background issues on folder previews.
//!
//! PERFORMANCE: Checks SQLite disk cache first (NVMe fast path) before calling
//! Shell API. For OneDrive paths, uses cached Shell API (IShellItemImageFactory)
//! instead of force extraction to avoid cloud filter driver latency.

use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Instant, UNIX_EPOCH};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Data returned from folder preview worker
pub struct FolderPreviewData {
    pub path: PathBuf,
    pub rgba_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Spawns a folder preview worker thread
///
/// # Arguments
/// * `rx` - Receiver for folder paths to process
/// * `tx` - Sender for processed preview data
/// * `ctx` - egui Context for repaint requests
/// * `disk_cache` - SQLite disk cache for persistent folder preview storage
pub fn spawn_folder_preview_worker(
    rx: Arc<Mutex<Receiver<PathBuf>>>,
    tx: Sender<FolderPreviewData>,
    ctx: egui::Context,
    disk_cache: Arc<ThumbnailDiskCache>,
) {
    std::thread::spawn(move || {
        unsafe {
            // SAFETY: Initializing COM for this worker thread
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut last_repaint = Instant::now();
        let mut last_ssd_state: Option<bool> = None;
        while let Some(path) = rx.lock().ok().and_then(|lock| lock.recv().ok()) {
            let is_ssd = crate::infrastructure::io_priority::is_ssd(&path);
            if last_ssd_state != Some(is_ssd) {
                let priority = if is_ssd {
                    crate::infrastructure::io_priority::IOPriority::Prefetch
                } else {
                    crate::infrastructure::io_priority::IOPriority::Background
                };
                crate::infrastructure::io_priority::set_thread_priority(priority);
                last_ssd_state = Some(is_ssd);
            }

            // Skip cloud-only OneDrive folders — Shell API can block on network I/O
            if crate::infrastructure::onedrive::is_onedrive_path(&path)
                && !crate::infrastructure::onedrive::is_locally_available(&path)
            {
                let _ = tx.send(FolderPreviewData {
                    path,
                    rgba_data: Vec::new(),
                    width: 0,
                    height: 0,
                });
                throttle_repaint(&ctx, &mut last_repaint);
                continue;
            }

            // FAST PATH: Check SQLite disk cache first (NVMe read, ~1ms)
            // Then verify the cache entry is still fresh by comparing
            // its created_at timestamp against the folder's last-write time.
            // This handles the case where files were changed while the app was closed.
            let cache_start = Instant::now();
            if let Some((rgba_data, width, height, created_at)) =
                disk_cache.get_folder_preview_cache(&path)
            {
                // Staleness check: if the folder was modified after we cached
                // the preview, the cache is stale → skip to Shell API.
                let is_stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                    .map(|dur| dur.as_secs() as i64 > created_at)
                    .unwrap_or(false);

                if !is_stale {
                    log::debug!(
                        "[FOLDER PREVIEW] DB HIT {:?} ({}x{}, {:.1}ms)",
                        path.file_name().unwrap_or_default(),
                        width,
                        height,
                        cache_start.elapsed().as_secs_f64() * 1000.0
                    );
                    let _ = tx.send(FolderPreviewData {
                        path,
                        rgba_data,
                        width,
                        height,
                    });
                    throttle_repaint(&ctx, &mut last_repaint);
                    continue;
                }
                log::debug!(
                    "[FOLDER PREVIEW] DB STALE {:?} (folder modified after cache) → Shell API",
                    path.file_name().unwrap_or_default(),
                );
            } else {
                log::debug!(
                    "[FOLDER PREVIEW] DB MISS {:?} ({:.1}ms) → Shell API",
                    path.file_name().unwrap_or_default(),
                    cache_start.elapsed().as_secs_f64() * 1000.0
                );
            }

            // SLOW PATH: Extract from Shell API
            // Strategy differs for OneDrive vs normal folders:
            // - OneDrive: Use cached Shell API first (fast), force_extract as fallback.
            //   WTS_FORCEEXTRACTION is very slow on OneDrive because the cloud filter
            //   driver adds latency to every file enumeration during preview generation.
            // - Normal: Use force_extract first to bypass potentially corrupted cache,
            //   then fall back to cached Shell API.
            let is_onedrive = crate::infrastructure::onedrive::is_onedrive_path(&path);

            let result = if is_onedrive {
                // OneDrive: prefer cached preview (fast) over force extraction (slow)
                match crate::infrastructure::windows::icons::get_folder_preview(&path) {
                    Ok(data) => Ok(data),
                    Err(_) => {
                        crate::infrastructure::windows::icons::force_extract_folder_preview(&path)
                    }
                }
            } else {
                // Normal folders: force_extract to avoid black background from corrupted cache
                match crate::infrastructure::windows::icons::force_extract_folder_preview(&path) {
                    Ok(data) => Ok(data),
                    Err(e) => {
                        log::warn!(
                            "[FOLDER PREVIEW] force_extract failed for {:?}: {}",
                            path, e
                        );
                        crate::infrastructure::windows::icons::get_folder_preview(&path)
                    }
                }
            };

            match result {
                Ok((rgba_data, width, height)) => {
                    // Write to disk cache in-band (WebP encode is fast for 256x256)
                    disk_cache.put_folder_preview_cache(&path, &rgba_data, width, height);

                    let _ = tx.send(FolderPreviewData {
                        path,
                        rgba_data,
                        width,
                        height,
                    });
                }
                Err(_) => {
                    let _ = tx.send(FolderPreviewData {
                        path,
                        rgba_data: Vec::new(),
                        width: 0,
                        height: 0,
                    });
                }
            }
            throttle_repaint(&ctx, &mut last_repaint);
        }

        crate::infrastructure::io_priority::reset_thread_priority();

        unsafe {
            // SAFETY: Cleanup COM for this thread
            CoUninitialize();
        }
    });
}

fn throttle_repaint(ctx: &egui::Context, last_repaint: &mut Instant) {
    if last_repaint.elapsed().as_millis() >= 33 {
        ctx.request_repaint();
        *last_repaint = Instant::now();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
