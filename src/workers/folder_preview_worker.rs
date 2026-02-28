//! Folder preview worker — custom composition + Shell API fallback
//!
//! PRIMARY: Extracts a thumbnail from the first media file inside the folder
//! using the existing 5-stage hybrid pipeline, then composes it with
//! folder_back.png and folder_front.png layers for a clean preview.
//!
//! FALLBACK: If no media is found or composition fails, falls back to
//! Windows Shell API (IThumbnailCache / IShellItemImageFactory).
//!
//! PERFORMANCE: Checks SQLite disk cache first (NVMe fast path, ~1ms).
//! Custom composition ~2ms vs Shell API 20-200ms.

use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::folder_compose::FolderComposer;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Data returned from folder preview worker
pub struct FolderPreviewData {
    pub path: PathBuf,
    pub rgba_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// M-19: RAII guard — ensures `CoUninitialize` runs even if the worker panics.
struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize(); }
    }
}

/// Spawns a folder preview worker thread
///
/// # Arguments
/// * `rx` - Receiver for folder paths to process
/// * `tx` - Sender for processed preview data
/// * `ctx` - egui Context for repaint requests
/// * `disk_cache` - SQLite disk cache for persistent folder preview storage
/// * `composer` - Pre-decoded folder layers for custom composition
pub fn spawn_folder_preview_worker(
    rx: crossbeam_channel::Receiver<PathBuf>,
    tx: Sender<FolderPreviewData>,
    ctx: egui::Context,
    disk_cache: Arc<ThumbnailDiskCache>,
    composer: Arc<FolderComposer>,
) {
    std::thread::spawn(move || {
        // M-19: RAII guard — CoUninitialize guaranteed on normal exit AND panic
        let _com = unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            ComGuard
        };

        // Empty DashMap — content thumbnail extraction doesn't need deletion tracking
        let empty_deletions = dashmap::DashMap::new();
        let mut last_repaint = Instant::now();
        let mut last_ssd_state: Option<bool> = None;

        while let Ok(path) = rx.recv() {
            if crate::infrastructure::windows::is_windows_system_path(&path.to_string_lossy()) {
                let (rgba_data, width, height) = composer.compose_empty();
                let _ = tx.send(FolderPreviewData {
                    path,
                    rgba_data,
                    width,
                    height,
                });
                throttle_repaint(&ctx, &mut last_repaint);
                continue;
            }

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
            let cache_start = Instant::now();
            if let Some((rgba_data, width, height, created_at)) =
                disk_cache.get_folder_preview_cache(&path)
            {
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
                    "[FOLDER PREVIEW] DB STALE {:?} (folder modified after cache)",
                    path.file_name().unwrap_or_default(),
                );
            } else {
                log::debug!(
                    "[FOLDER PREVIEW] DB MISS {:?} ({:.1}ms)",
                    path.file_name().unwrap_or_default(),
                    cache_start.elapsed().as_secs_f64() * 1000.0
                );
            }

            // SLOW PATH: Custom composition (primary) → Shell API (fallback)
            let io_priority = if is_ssd {
                crate::infrastructure::io_priority::IOPriority::Prefetch
            } else {
                crate::infrastructure::io_priority::IOPriority::Background
            };

            // Try custom composition first; fall back to empty (back+front only) for folders
            // without media. Shell API is no longer used — we always use our own folder assets.
            let (rgba_data, width, height) =
                try_custom_compose(&path, &composer, io_priority, &empty_deletions)
                    .unwrap_or_else(|| composer.compose_empty());

            disk_cache.put_folder_preview_cache(&path, &rgba_data, width, height);
            let _ = tx.send(FolderPreviewData {
                path,
                rgba_data,
                width,
                height,
            });
            throttle_repaint(&ctx, &mut last_repaint);
        }

        crate::infrastructure::io_priority::reset_thread_priority();
        // _com dropped here — CoUninitialize() guaranteed by RAII
    });
}

/// PRIMARY: Find a media file inside the folder, extract its thumbnail via the
/// 5-stage pipeline, then compose with folder back/front layers.
fn try_custom_compose(
    folder_path: &std::path::Path,
    composer: &FolderComposer,
    priority: crate::infrastructure::io_priority::IOPriority,
    empty_deletions: &dashmap::DashMap<PathBuf, ()>,
) -> Option<(Vec<u8>, u32, u32)> {
    let compose_start = Instant::now();

    // 1. Find first image/video inside the folder
    let media_path = crate::infrastructure::windows::find_folder_preview_item(folder_path)?;

    // 2. Extract content thumbnail using the existing 5-stage hybrid pipeline
    let (content_rgba, content_w, content_h) =
        crate::workers::thumbnail::extraction::generate_thumbnail_hybrid(
            &media_path,
            priority,
            empty_deletions,
        )?;

    // 3. Compose: back → content → front
    let result = composer.compose(&content_rgba, content_w, content_h);

    if result.is_some() {
        log::debug!(
            "[FOLDER PREVIEW] Custom compose SUCCESS {:?} via {:?} ({:.1}ms)",
            folder_path.file_name().unwrap_or_default(),
            media_path.file_name().unwrap_or_default(),
            compose_start.elapsed().as_secs_f64() * 1000.0
        );
    } else {
        log::debug!(
            "[FOLDER PREVIEW] Custom compose FAILED for {:?} (compose returned None)",
            folder_path.file_name().unwrap_or_default(),
        );
    }

    result
}

fn throttle_repaint(ctx: &egui::Context, last_repaint: &mut Instant) {
    if last_repaint.elapsed().as_millis() >= 33 {
        ctx.request_repaint();
        *last_repaint = Instant::now();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
