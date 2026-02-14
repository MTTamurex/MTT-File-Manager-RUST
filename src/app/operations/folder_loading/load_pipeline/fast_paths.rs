use crate::domain::file_entry::{is_archive_extension, FileEntry, SyncStatus};
use crate::infrastructure::adaptive_batch::AdaptiveBatchTracker;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::windows::{is_shell_navigation_path, list_shell_folder};
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) fn try_handle_fast_paths(
    my_gen: usize,
    gen_clone: &Arc<AtomicUsize>,
    current_path: &str,
    force_refresh: bool,
    base_path: &str,
    base_path_buf: &PathBuf,
    is_ssd: bool,
    is_onedrive_base: bool,
    batch_size: &mut usize,
    batch_tracker: &mut AdaptiveBatchTracker,
    batch_start: &mut Instant,
    file_entry_sender: &Sender<(usize, Vec<FileEntry>)>,
    ctx: &egui::Context,
    disk_cache: &Arc<ThumbnailDiskCache>,
    directory_cache: &Arc<DirectoryCache>,
    directory_index_opt: &Option<Arc<DirectoryIndex>>,
) -> bool {
    let directory_mtime_ms = |path: &PathBuf| -> u64 {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    };

    // Phase 1: Instant feedback from DirectoryCache (all local disks).
    {
        // DriveWatcher monitors the ENTIRE drive and proactively invalidates
        // both DirectoryCache and DirectoryIndex for ANY change on the drive.
        // No fs::metadata() mtime check needed — if the cache has data, it's valid.
        if let Some((cached_entries, cached_at_ms)) = directory_cache.get_with_meta(base_path_buf) {
            // Fail-safe against missed watcher events: validate folder mtime.
            // Skip for OneDrive to avoid potential blocking metadata calls.
            if !is_onedrive_base {
                let dir_mtime_ms = directory_mtime_ms(base_path_buf);
                if dir_mtime_ms > cached_at_ms {
                    log::debug!(
                        "[FOLDER-LOADING] DirectoryCache stale for {:?} (dir_mtime_ms={} > cached_at_ms={}), invalidating",
                        base_path_buf, dir_mtime_ms, cached_at_ms
                    );
                    directory_cache.invalidate(base_path_buf);
                    if let Some(di) = directory_index_opt {
                        let _ = di.invalidate(base_path_buf);
                    }
                } else {
                    log::debug!(
                        "[FOLDER-LOADING] Phase 1: Cache hit for {:?} - {} entries, sending to UI immediately",
                        base_path_buf,
                        cached_entries.len()
                    );

                    // For cached OneDrive entries, keep SyncStatus::None (unknown)
                    // until fresh disk enumeration provides authoritative status.
                    let entries_to_send = cached_entries;

                    // INSTANT RETURN: Send cached entries immediately (0ms navigation)
                    let mut offset = 0;
                    while offset < entries_to_send.len() {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return true;
                        }
                        let end = (offset + *batch_size).min(entries_to_send.len());
                        let chunk = entries_to_send[offset..end].to_vec();
                        let _ = file_entry_sender.send((my_gen, chunk));
                        ctx.request_repaint();
                        batch_tracker
                            .record_batch(std::time::Instant::now().elapsed(), end - offset);
                        *batch_size = batch_tracker.batch_size();
                        offset = end;
                    }
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();

                    // Phase 2: HDD SILENCE - Trust cache + file watcher
                    // The file watcher (ReadDirectoryChangesW) passively monitors the current
                    // directory and invalidates the cache on changes. We don't need to poll
                    // the filesystem to check for modifications — the watcher handles this.
                    // This eliminates std::fs::metadata() syscalls on HDD per navigation.
                    log::debug!(
                        "[FOLDER-LOADING] Phase 2: Cache valid, trusting watcher for {:?} - HDD silence maintained",
                        base_path_buf
                    );
                    return true;
                }
            } else {
                log::debug!(
                    "[FOLDER-LOADING] Phase 1: Cache hit for {:?} - {} entries, sending to UI immediately",
                    base_path_buf,
                    cached_entries.len()
                );

                // For cached OneDrive entries, keep SyncStatus::None (unknown)
                // until fresh disk enumeration provides authoritative status.
                let entries_to_send = cached_entries;

                // INSTANT RETURN: Send cached entries immediately (0ms navigation)
                let mut offset = 0;
                while offset < entries_to_send.len() {
                    if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                        return true;
                    }
                    let end = (offset + *batch_size).min(entries_to_send.len());
                    let chunk = entries_to_send[offset..end].to_vec();
                    let _ = file_entry_sender.send((my_gen, chunk));
                    ctx.request_repaint();
                    batch_tracker.record_batch(std::time::Instant::now().elapsed(), end - offset);
                    *batch_size = batch_tracker.batch_size();
                    offset = end;
                }
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();

                // Phase 2: HDD SILENCE - Trust cache + file watcher
                // The file watcher (ReadDirectoryChangesW) passively monitors the current
                // directory and invalidates the cache on changes. We don't need to poll
                // the filesystem to check for modifications — the watcher handles this.
                // This eliminates std::fs::metadata() syscalls on HDD per navigation.
                log::debug!(
                    "[FOLDER-LOADING] Phase 2: Cache valid, trusting watcher for {:?} - HDD silence maintained",
                    base_path_buf
                );
                return true;
            }
        } else {
            log::debug!(
                "[FOLDER-LOADING] Phase 1: Cache miss for {:?}, proceeding to Phase 3 (disk load)",
                base_path_buf
            );
        }
    }

    log::debug!(
        "[FOLDER-LOADING] Phase 3: Starting disk scan for {:?} (batch_size={}, is_ssd={})",
        current_path,
        *batch_size,
        is_ssd
    );

    // Check if we are navigating a virtual Shell folder (like an archive)
    if is_shell_navigation_path(&PathBuf::from(base_path), false) {
        log::debug!(
            "[FOLDER-LOADING] Shell navigation detected for {:?}",
            base_path
        );
        match list_shell_folder(&PathBuf::from(base_path)) {
            Ok(shell_items) => {
                log::debug!(
                    "[FOLDER-LOADING] Shell folder listed OK: {} items for {:?}",
                    shell_items.len(),
                    base_path
                );
                if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                    let _ = file_entry_sender.send((my_gen, shell_items));
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();
                    return true;
                }
            }
            Err(e) => {
                log::error!(
                    "[FOLDER-LOADING] Shell folder FAILED for {:?}: {:?}",
                    base_path,
                    e
                );
            }
        }
    } else {
        log::debug!(
            "[FOLDER-LOADING] NOT detected as shell path: {:?}",
            base_path
        );
    }

    if !force_refresh && !is_onedrive_base {
        if let Some(di) = directory_index_opt {
            let base = PathBuf::from(base_path);
            // SAFETY CHECK: Verify directory mtime before trusting cached index
            // The DriveWatcher may not have been active when files were added
            // (e.g., during startup delay or when the app was closed).
            // A single metadata() call is cheap compared to serving stale data.
            if let Some((meta, indexed_files)) = di.get_directory(&base) {
                // Validate: Check if directory mtime is newer than last_scan
                // CRITICAL: For OneDrive paths, skip mtime validation (can block indefinitely on cloud-only dirs)
                // Trust the DriveWatcher and explicit invalidation instead
                let dir_modified = if crate::infrastructure::onedrive::is_onedrive_path(&base) {
                    0 // Skip mtime check for OneDrive - trust watcher
                } else {
                    std::fs::metadata(&base)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                };

                if dir_modified > meta.last_scan {
                    // Index is stale - directory was modified after last scan
                    log::debug!(
                        "[FOLDER-LOADING] DirectoryIndex stale for {:?} (dir_mtime={} > index_time={}), invalidating",
                        base, dir_modified, meta.last_scan
                    );
                    let _ = di.invalidate(&base);
                    // Fall through to disk scan below
                } else {
                    log::debug!(
                        "[FOLDER-LOADING] Using DirectoryIndex (pre-built index) for {:?}",
                        base
                    );
                    let mut entries: Vec<FileEntry> = indexed_files
                        .into_iter()
                        .filter(|f| !f.name.starts_with('.'))
                        .map(|f| {
                            let is_archive = is_archive_extension(&f.name);
                            let is_dir = f.is_dir || is_archive;
                            FileEntry {
                                path: base.join(&f.name),
                                name: f.name,
                                is_dir,
                                size: if is_dir && !is_archive { 0 } else { f.size },
                                modified: f.modified,
                                folder_cover: None,
                                drive_info: None,
                                sync_status: SyncStatus::None,
                                deletion_date: None,
                                recycle_original_path: None,
                            }
                        })
                        .collect();

                    let folders: Vec<PathBuf> = entries
                        .iter()
                        .filter(|e| e.is_dir)
                        .map(|e| e.path.clone())
                        .collect();
                    if !folders.is_empty() {
                        let covers = disk_cache.get_folder_covers(&folders);
                        for entry in entries.iter_mut() {
                            if entry.is_dir {
                                if let Some(cover) = covers.get(&entry.path) {
                                    entry.folder_cover = Some(cover.clone());
                                }
                            }
                        }
                    }

                    directory_cache.put(base.clone(), entries.clone());

                    let mut offset = 0;
                    while offset < entries.len() {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return true;
                        }
                        let end = (offset + *batch_size).min(entries.len());
                        let chunk = entries[offset..end].to_vec();
                        let _ = file_entry_sender.send((my_gen, chunk));
                        ctx.request_repaint();
                        batch_tracker.record_batch(batch_start.elapsed(), end - offset);
                        let _ = batch_tracker.batch_size();
                        *batch_start = std::time::Instant::now();
                        offset = end;
                    }
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();
                    return true;
                } // end else (index not stale)
            } // end if let Some((meta, indexed_files))
        } // end if let Some(di)
    } // end if !force_refresh && !is_onedrive_base

    if !force_refresh {
        // DriveWatcher pre-invalidates cache — no mtime check needed
        let base_path_buf_owned = PathBuf::from(base_path);
        if let Some((mut cached_entries, cached_at_ms)) =
            directory_cache.get_with_meta(&base_path_buf_owned)
        {
            if !is_onedrive_base {
                let dir_mtime_ms = directory_mtime_ms(&base_path_buf_owned);
                if dir_mtime_ms > cached_at_ms {
                    log::debug!(
                        "[FOLDER-LOADING] Secondary DirectoryCache stale for {:?} (dir_mtime_ms={} > cached_at_ms={}), invalidating",
                        base_path_buf_owned, dir_mtime_ms, cached_at_ms
                    );
                    directory_cache.invalidate(&base_path_buf_owned);
                    if let Some(di) = directory_index_opt {
                        let _ = di.invalidate(&base_path_buf_owned);
                    }
                    return false;
                }
            }

            log::debug!(
                "[FOLDER-LOADING] Using secondary DirectoryCache for {:?}",
                base_path
            );
            let mut changed = false;
            for entry in cached_entries.iter_mut() {
                if !entry.is_dir && is_archive_extension(&entry.name) {
                    entry.is_dir = true;
                    changed = true;
                }
            }

            if changed {
                directory_cache.put(PathBuf::from(base_path), cached_entries.clone());
            }
            let mut offset = 0;
            while offset < cached_entries.len() {
                if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                    return true;
                }
                let end = (offset + *batch_size).min(cached_entries.len());
                let chunk = cached_entries[offset..end].to_vec();
                let _ = file_entry_sender.send((my_gen, chunk));
                ctx.request_repaint();
                batch_tracker.record_batch(std::time::Instant::now().elapsed(), end - offset);
                *batch_size = batch_tracker.batch_size();
                offset = end;
            }
            let _ = file_entry_sender.send((my_gen, Vec::new()));
            ctx.request_repaint();

            // PERFORMANCE: Don't prefetch when serving from cache.
            // Prefetch only runs after actual disk enumeration (first visit).
            // Subdirectories are likely already cached from previous visits.
            // This eliminates 5x background directory enumerations on HDD.
            return true;
        }
    }

    false
}
