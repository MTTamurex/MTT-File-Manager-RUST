use crate::domain::file_entry::{is_archive_extension, FileEntry};
use crate::infrastructure::adaptive_batch::AdaptiveBatchTracker;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_dirty_registry::DirectoryDirtyRegistry;
use crate::infrastructure::directory_index::{DirectoryIndex, IndexedFile};
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::onedrive;
use eframe::egui;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
use windows::Win32::Storage::FileSystem::*;

fn classify_directory_open_error(
    path: PathBuf,
    error: windows::core::Error,
) -> crate::app::state::FolderLoadError {
    let code = error.code();
    if code == ERROR_ACCESS_DENIED.to_hresult() {
        crate::app::state::FolderLoadError::access_denied(path)
    } else if code == ERROR_PATH_NOT_FOUND.to_hresult() || code == ERROR_FILE_NOT_FOUND.to_hresult()
    {
        crate::app::state::FolderLoadError::not_found(path)
    } else {
        crate::app::state::FolderLoadError::other(path, error.to_string())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_tier3_fallback(
    my_gen: usize,
    gen_clone: &Arc<AtomicUsize>,
    scan_start: &Instant,
    current_path: &str,
    base_path: &str,
    is_onedrive_base: bool,
    batch_size: &mut usize,
    batch_tracker: &mut AdaptiveBatchTracker,
    batch_start: &mut Instant,
    batch: &mut Vec<FileEntry>,
    all_entries_disk: &mut Vec<FileEntry>,
    file_entry_sender: &Sender<(usize, Vec<FileEntry>)>,
    folder_load_failure_sender: &Sender<(usize, crate::app::state::FolderLoadError)>,
    ctx: &egui::Context,
    _disk_cache: &Arc<ThumbnailDiskCache>,
    app_state_db: &Arc<AppStateDb>,
    directory_cache: &Arc<DirectoryCache>,
    directory_dirty_registry: &Arc<DirectoryDirtyRegistry>,
    directory_index_opt: &Option<Arc<DirectoryIndex>>,
    show_hidden: bool,
) {
    // TIER 3: Standard FindFirstFileW fallback (last resort)
    // CRITICAL FIX: For OneDrive folders, use timeout-protected enumeration
    // to prevent 30-60s blocking on folders with cloud-only files.
    let is_onedrive = is_onedrive_base;

    if is_onedrive {
        // Use timeout-protected directory reading for OneDrive.
        log::debug!(
            "[FOLDER-LOADING] Using timeout-protected directory enumeration for OneDrive: {:?}",
            base_path
        );
        let onedrive_enum_start = std::time::Instant::now();
        match onedrive::onedrive_read_directory(&PathBuf::from(base_path)) {
            onedrive::IoTimeoutResult::Ok(entries) => {
                log::debug!(
                    "[PERF] OneDrive enum complete: {:?} items={} elapsed={}ms",
                    base_path,
                    entries.len(),
                    onedrive_enum_start.elapsed().as_millis()
                );
                batch.clear();
                batch.reserve((*batch_size).max(64));

                for (filename, attrs, size, modified) in entries {
                    if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                        break;
                    }

                    let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                    let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                    let is_special = matches!(
                        filename.to_lowercase().as_str(),
                        "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
                    );

                    if (show_hidden || !is_hidden)
                        && !is_system
                        && !is_special
                        && !filename.starts_with('.')
                    {
                        let mut is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
                        let full_path = PathBuf::from(base_path).join(&filename);

                        let is_archive = is_archive_extension(&filename);
                        if !is_dir && is_archive {
                            is_dir = true;
                        }

                        let file_size = if is_dir && !is_archive { 0 } else { size };
                        let sync_status = onedrive::get_sync_status(attrs, true);

                        let entry = FileEntry {
                            path: full_path,
                            name: filename,
                            is_dir,
                            size: file_size,
                            modified,
                            created: None,
                            folder_cover: None,
                            drive_info: None,
                            sync_status,
                            is_hidden,
                            recycle_bin: None,
                        };

                        all_entries_disk.push(entry.clone());
                        batch.push(entry);

                        // Send adaptive batches to improve first paint.
                        if batch.len() >= *batch_size {
                            let batch_len = batch.len();
                            let _ = file_entry_sender.send((my_gen, std::mem::take(batch)));
                            batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                            *batch_size = batch_tracker.batch_size();
                            *batch_start = std::time::Instant::now();
                            batch.reserve((*batch_size).max(64));
                            ctx.request_repaint();
                        }
                    }
                }

                // Send remaining entries.
                if !batch.is_empty() {
                    let batch_len = batch.len();
                    let _ = file_entry_sender.send((my_gen, std::mem::take(batch)));
                    batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                }

                // Signal completion.
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();

                // Populate caches so subsequent OneDrive navigations are instant.
                if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                    directory_cache.put(PathBuf::from(base_path), all_entries_disk.clone());
                    directory_dirty_registry.clear_dirty(PathBuf::from(base_path).as_path());

                    if !show_hidden {
                        if let Some(di) = directory_index_opt {
                            let indexed: Vec<IndexedFile> = all_entries_disk
                                .iter()
                                .map(|e| IndexedFile {
                                    name: e.name.clone(),
                                    size: e.size,
                                    modified: e.modified,
                                    is_dir: e.is_dir,
                                    created: e.created.unwrap_or(0),
                                })
                                .collect();
                            let _ = di.put_directory(
                                &PathBuf::from(base_path),
                                &indexed,
                                scan_start.elapsed().as_millis() as u64,
                            );
                        }
                    }
                }

                log::debug!(
                    "[FOLDER-LOADING] OneDrive directory enumeration completed successfully in {}ms (visible_items={})",
                    onedrive_enum_start.elapsed().as_millis(),
                    all_entries_disk.len()
                );
                return;
            }
            onedrive::IoTimeoutResult::Timeout => {
                log::error!(
                    "[FOLDER-LOADING] CRITICAL: OneDrive directory enumeration timed out after 5s for {:?}",
                    base_path
                );
                // CRITICAL FIX: Do NOT fall through to standard FindFirstFileW.
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
                log::warn!("[FOLDER-LOADING] OneDrive enumeration timed out - sent empty results");
                return;
            }
            onedrive::IoTimeoutResult::Err(_) => {
                log::warn!(
                    "[FOLDER-LOADING] Error in OneDrive directory enumeration, falling back to standard"
                );
                // On error (not timeout), fall through to standard Win32.
            }
        }
    }

    // Standard FindFirstFileW (for non-OneDrive or fallback).
    let search_path = if base_path.ends_with('\\') {
        format!("{}*", base_path)
    } else {
        format!("{}\\*", base_path)
    };
    let wide_path: Vec<u16> = search_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut find_data = WIN32_FIND_DATAW::default();

    unsafe {
        // SAFETY: `wide_path` is a null-terminated UTF-16 string buffer.
        match FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
            Ok(handle) => {
                loop {
                    if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                        break;
                    }

                    let len = find_data
                        .cFileName
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(find_data.cFileName.len());
                    let filename = std::ffi::OsString::from_wide(&find_data.cFileName[0..len])
                        .to_string_lossy()
                        .into_owned();

                    if filename != "." && filename != ".." {
                        let attrs = find_data.dwFileAttributes;
                        let full_path = PathBuf::from(base_path).join(&filename);
                        let extended_attrs = attrs;

                        let is_hidden = (extended_attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                        let is_system = (extended_attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                        let is_special = matches!(
                            filename.to_lowercase().as_str(),
                            "desktop.ini"
                                | "thumbs.db"
                                | "$recycle.bin"
                                | "system volume information"
                        );

                        if (show_hidden || !is_hidden)
                            && !is_system
                            && !is_special
                            && !filename.starts_with('.')
                        {
                            let mut is_dir = (extended_attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

                            // Treat archive files as navigable folders.
                            let is_archive = is_archive_extension(&filename);
                            if !is_dir && is_archive {
                                is_dir = true;
                            }

                            let size = if is_dir && !is_archive {
                                0
                            } else {
                                ((find_data.nFileSizeHigh as u64) << 32)
                                    | (find_data.nFileSizeLow as u64)
                            };

                            let ft = find_data.ftLastWriteTime;
                            let windows_ticks =
                                ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                            let modified = if windows_ticks > 116444736000000000 {
                                (windows_ticks - 116444736000000000) / 10_000_000
                            } else {
                                0
                            };

                            let ft_created = find_data.ftCreationTime;
                            let created_ticks = ((ft_created.dwHighDateTime as u64) << 32)
                                | (ft_created.dwLowDateTime as u64);
                            let created = if created_ticks > 116444736000000000 {
                                Some((created_ticks - 116444736000000000) / 10_000_000)
                                    .filter(|&c| c > 0)
                            } else {
                                None
                            };

                            let sync_status =
                                onedrive::get_sync_status(extended_attrs, is_onedrive);

                            let entry = FileEntry {
                                path: full_path,
                                name: filename,
                                is_dir,
                                size,
                                modified,
                                created,
                                folder_cover: None,
                                drive_info: None,
                                sync_status,
                                is_hidden,
                                recycle_bin: None,
                            };

                            all_entries_disk.push(entry.clone());
                            batch.push(entry);

                            // If batch is full, send and clear.
                            if batch.len() >= *batch_size {
                                let folders: Vec<PathBuf> = batch
                                    .iter()
                                    .filter(|e| e.is_dir)
                                    .map(|e| e.path.clone())
                                    .collect();

                                if !folders.is_empty() {
                                    let covers = app_state_db.get_folder_covers(&folders);
                                    for item in batch.iter_mut() {
                                        if item.is_dir {
                                            if let Some(cover) = covers.get(&item.path) {
                                                item.folder_cover = Some(cover.clone());
                                            }
                                        }
                                    }
                                }

                                let batch_len = batch.len();
                                let _ = file_entry_sender.send((my_gen, std::mem::take(batch)));
                                batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                                *batch_size = batch_tracker.batch_size();
                                *batch_start = std::time::Instant::now();
                                ctx.request_repaint();
                            }
                        }
                    }

                    if FindNextFileW(handle, &mut find_data).is_err() {
                        break;
                    }
                }
                let _ = FindClose(handle);
            }
            Err(error) => {
                let base_path_buf = PathBuf::from(base_path);
                let failure = if !crate::infrastructure::onedrive::fast_is_dir(&base_path_buf) {
                    log::warn!(
                        "[FOLDER-LOADING] Directory vanished during load: {:?}",
                        base_path_buf
                    );
                    crate::app::state::FolderLoadError::not_found(base_path_buf)
                } else {
                    let failure = classify_directory_open_error(base_path_buf, error);
                    log::warn!(
                        "[FOLDER-LOADING] Directory enumeration failed: kind={:?} path={} message={:?}",
                        failure.kind,
                        failure.path.display(),
                        failure.message
                    );
                    failure
                };
                let _ = folder_load_failure_sender.send((my_gen, failure));
                ctx.request_repaint();
                return;
            }
        }
    }

    // Send remaining (last batch) if generation is still valid.
    if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
        let folders: Vec<PathBuf> = batch
            .iter()
            .filter(|e| e.is_dir)
            .map(|e| e.path.clone())
            .collect();

        if !folders.is_empty() {
            let covers = app_state_db.get_folder_covers(&folders);
            for item in batch.iter_mut() {
                if item.is_dir {
                    if let Some(cover) = covers.get(&item.path) {
                        item.folder_cover = Some(cover.clone());
                    }
                }
            }
        }

        let batch_len = batch.len();
        let _ = file_entry_sender.send((my_gen, std::mem::take(batch)));
        batch_tracker.record_batch(batch_start.elapsed(), batch_len);
        ctx.request_repaint();
    }

    // Send empty vector to signal end of loading.
    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
        let scan_elapsed = scan_start.elapsed();
        log::debug!(
            "[PERF] Folder scan complete: {:?} took {:.2}s",
            current_path,
            scan_elapsed.as_secs_f64()
        );
        let _ = file_entry_sender.send((my_gen, Vec::new()));
        ctx.request_repaint();
    }

    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
        // Cache storage for instant future navigation.
        directory_cache.put(PathBuf::from(base_path), all_entries_disk.clone());
        directory_dirty_registry.clear_dirty(PathBuf::from(base_path).as_path());

        if !show_hidden {
            if let Some(di) = directory_index_opt {
                let indexed: Vec<IndexedFile> = all_entries_disk
                    .iter()
                    .map(|e| IndexedFile {
                        name: e.name.clone(),
                        size: e.size,
                        modified: e.modified,
                        is_dir: e.is_dir,
                        created: e.created.unwrap_or(0),
                    })
                    .collect();
                let _ = di.put_directory(
                    &PathBuf::from(base_path),
                    &indexed,
                    scan_start.elapsed().as_millis() as u64,
                );
            }
        }
    }
}
