use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::infrastructure::adaptive_batch::{AdaptiveBatchConfig, AdaptiveBatchTracker};
use crate::infrastructure::directory_index::IndexedFile;
use crate::infrastructure::io_priority;
use crate::infrastructure::ntfs_reader;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows::{is_shell_navigation_path, list_shell_folder};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering as AtomicOrdering;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::*;

impl ImageViewerApp {
    pub(super) fn start_folder_load_pipeline(&mut self, force_refresh: bool) {
        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        let disk_cache = self.disk_cache.clone();
        let directory_cache = self.directory_cache.clone();
        // Use existing directory_cache for cache-first strategy
        let directory_index_opt = self.directory_index.clone();
        let _prefetch_sender = self.prefetch_sender.clone();

        // STREAMING BATCH LOADING: Adaptive batch size based on disk type
        std::thread::spawn(move || {
            let scan_start = std::time::Instant::now();

            let base_path = if current_path.len() == 2 && current_path.ends_with(':') {
                format!("{}\\", current_path)
            } else {
                current_path.clone()
            };

            let is_ssd = io_priority::is_ssd(&PathBuf::from(&current_path));
            let config = AdaptiveBatchConfig {
                is_ssd,
                total_items: directory_index_opt
                    .as_ref()
                    .and_then(|di| di.get_directory(&PathBuf::from(&base_path)))
                    .map(|(meta, _)| meta.file_count),
            };
            let mut batch_tracker = AdaptiveBatchTracker::new(config);
            let mut batch_size = batch_tracker.batch_size();

            // STALE-WHILE-REVALIDATE STRATEGY: Instant feedback with debounce
            // NOTE: Only used for HDDs - SSDs bypass cache entirely for raw speed
            let base_path_buf = PathBuf::from(&base_path);
            // PERFORMANCE: Only use is_onedrive_path() which is string-based (no I/O)
            // path_has_cloud_attributes() was removed because GetFileAttributesW can BLOCK
            // indefinitely on cloud-only OneDrive folders
            let is_onedrive_base = onedrive::is_onedrive_path(&base_path_buf);

            // Phase 1: Instant Feedback (The Cache Hit) - HDD ONLY
            if !is_ssd {
                // DriveWatcher monitors the ENTIRE drive and proactively invalidates
                // both DirectoryCache and DirectoryIndex for ANY change on the drive.
                // No fs::metadata() mtime check needed — if the cache has data, it's valid.
                if let Some(cached_entries) = directory_cache.get(&base_path_buf) {
                    eprintln!("[FOLDER-LOADING] Phase 1: Cache hit for {:?} - {} entries, sending to UI immediately",
                        base_path_buf, cached_entries.len());

                    // For cached OneDrive entries, keep SyncStatus::None (unknown)
                    // until fresh disk enumeration provides authoritative status.
                    let entries_to_send = cached_entries;

                    // INSTANT RETURN: Send cached entries immediately (0ms navigation)
                    let mut offset = 0;
                    while offset < entries_to_send.len() {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }
                        let end = (offset + batch_size).min(entries_to_send.len());
                        let chunk = entries_to_send[offset..end].to_vec();
                        let _ = file_entry_sender.send((my_gen, chunk));
                        ctx.request_repaint();
                        batch_tracker
                            .record_batch(std::time::Instant::now().elapsed(), end - offset);
                        batch_size = batch_tracker.batch_size();
                        offset = end;
                    }
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();

                    // Phase 2: HDD SILENCE - Trust cache + file watcher
                    // The file watcher (ReadDirectoryChangesW) passively monitors the current
                    // directory and invalidates the cache on changes. We don't need to poll
                    // the filesystem to check for modifications — the watcher handles this.
                    // This eliminates std::fs::metadata() syscalls on HDD per navigation.
                    eprintln!("[FOLDER-LOADING] Phase 2: Cache valid, trusting watcher for {:?} - HDD silence maintained", base_path_buf);
                    return;
                } else {
                    eprintln!("[FOLDER-LOADING] Phase 1: Cache miss for {:?}, proceeding to Phase 3 (disk load)", base_path_buf);
                }
            } else {
                eprintln!("[FOLDER-LOADING] SSD detected - bypassing cache for raw disk speed");
            }

            eprintln!(
                "[FOLDER-LOADING] Phase 3: Starting disk scan for {:?} (batch_size={}, is_ssd={})",
                current_path, batch_size, is_ssd
            );

            let mut batch = Vec::with_capacity(batch_size);
            let mut all_entries_disk: Vec<FileEntry> = Vec::new();
            let mut batch_start = std::time::Instant::now();

            // Check if we are navigating a virtual Shell folder (like a ZIP)
            if is_shell_navigation_path(&PathBuf::from(&base_path), false) {
                if let Ok(shell_items) = list_shell_folder(&PathBuf::from(&base_path)) {
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, shell_items));
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                        return;
                    }
                }
            }

            if !force_refresh && !is_onedrive_base {
                if let Some(di) = &directory_index_opt {
                    let base = PathBuf::from(&base_path);
                    // SAFETY CHECK: Verify directory mtime before trusting cached index
                    // The DriveWatcher may not have been active when files were added
                    // (e.g., during startup delay or when the app was closed).
                    // A single metadata() call is cheap compared to serving stale data.
                    if let Some((meta, indexed_files)) = di.get_directory(&base) {
                        // Validate: Check if directory mtime is newer than last_scan
                        // CRITICAL: For OneDrive paths, skip mtime validation (can block indefinitely on cloud-only dirs)
                        // Trust the DriveWatcher and explicit invalidation instead
                        let dir_modified =
                            if crate::infrastructure::onedrive::is_onedrive_path(&base) {
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
                            eprintln!("[FOLDER-LOADING] DirectoryIndex stale for {:?} (dir_mtime={} > index_time={}), invalidating",
                                base, dir_modified, meta.last_scan);
                            let _ = di.invalidate(&base);
                            // Fall through to disk scan below
                        } else {
                            eprintln!(
                                "[FOLDER-LOADING] Using DirectoryIndex (pre-built index) for {:?}",
                                base
                            );
                            let mut entries: Vec<FileEntry> = indexed_files
                                .into_iter()
                                .filter(|f| !f.name.starts_with('.'))
                                .map(|f| {
                                    let is_zip = f.name.to_lowercase().ends_with(".zip");
                                    let is_dir = f.is_dir || is_zip;
                                    FileEntry {
                                        path: base.join(&f.name),
                                        name: f.name,
                                        is_dir,
                                        size: if is_dir && !is_zip { 0 } else { f.size },
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

                            // Only cache for HDDs - SSDs bypass cache
                            if !is_ssd {
                                directory_cache.put(base.clone(), entries.clone());
                            }

                            let mut offset = 0;
                            while offset < entries.len() {
                                if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                    return;
                                }
                                let end = (offset + batch_size).min(entries.len());
                                let chunk = entries[offset..end].to_vec();
                                let _ = file_entry_sender.send((my_gen, chunk));
                                ctx.request_repaint();
                                batch_tracker.record_batch(batch_start.elapsed(), end - offset);
                                let _ = batch_tracker.batch_size();
                                batch_start = std::time::Instant::now();
                                offset = end;
                            }
                            let _ = file_entry_sender.send((my_gen, Vec::new()));
                            ctx.request_repaint();
                            return;
                        } // end else (index not stale)
                    } // end if let Some((meta, indexed_files))
                } // end if let Some(di)
            } // end if !force_refresh && !is_onedrive_base

            if !force_refresh {
                // DriveWatcher pre-invalidates cache — no mtime check needed
                if let Some(mut cached_entries) = directory_cache.get(&PathBuf::from(&base_path)) {
                    eprintln!(
                        "[FOLDER-LOADING] Using secondary DirectoryCache for {:?}",
                        base_path
                    );
                    let mut changed = false;
                    for entry in cached_entries.iter_mut() {
                        if !entry.is_dir && entry.name.to_lowercase().ends_with(".zip") {
                            entry.is_dir = true;
                            changed = true;
                        }
                    }

                    if changed {
                        // Only cache for HDDs - SSDs bypass cache
                        if !is_ssd {
                            directory_cache.put(PathBuf::from(&base_path), cached_entries.clone());
                        }
                    }
                    let mut offset = 0;
                    while offset < cached_entries.len() {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            return;
                        }
                        let end = (offset + batch_size).min(cached_entries.len());
                        let chunk = cached_entries[offset..end].to_vec();
                        let _ = file_entry_sender.send((my_gen, chunk));
                        ctx.request_repaint();
                        batch_tracker
                            .record_batch(std::time::Instant::now().elapsed(), end - offset);
                        batch_size = batch_tracker.batch_size();
                        offset = end;
                    }
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();

                    // PERFORMANCE: Don't prefetch when serving from cache.
                    // Prefetch only runs after actual disk enumeration (first visit).
                    // Subdirectories are likely already cached from previous visits.
                    // This eliminates 5x background directory enumerations on HDD.
                    return;
                }
            }

            // OPTIMIZATION: Tiered disk reading strategy
            // Priority: 1) NTFS native API, 2) HDD-optimized FindFirstFileExW, 3) Standard FindFirstFileW
            let is_hdd = !is_ssd;
            let ntfs_api_available = ntfs_reader::is_available();

            // Track if we successfully used an optimized path
            let used_optimized_path = false;

            // TIER 1: Try NTFS native API first (fastest for NTFS drives)
            if is_hdd && ntfs_api_available {
                eprintln!("[FOLDER-LOADING] TIER 1: Trying NTFS native API (NtQueryDirectoryFile) for {:?}", base_path);
                if let Some(entries) = ntfs_reader::read_directory_fast(&PathBuf::from(&base_path))
                {
                    for dir_entry in entries {
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                            break;
                        }
                        let is_hidden = (dir_entry.attributes & 0x02) != 0;
                        let is_system = (dir_entry.attributes & 0x04) != 0;
                        let is_special = matches!(
                            dir_entry.name.to_lowercase().as_str(),
                            "desktop.ini"
                                | "thumbs.db"
                                | "$recycle.bin"
                                | "system volume information"
                        );
                        if !is_hidden
                            && !is_system
                            && !is_special
                            && !dir_entry.name.starts_with('.')
                        {
                            let full_path = PathBuf::from(&base_path).join(&dir_entry.name);
                            let mut is_dir = dir_entry.is_dir;
                            if !is_dir && dir_entry.name.to_lowercase().ends_with(".zip") {
                                is_dir = true;
                            }
                            let sync_status =
                                onedrive::get_sync_status(dir_entry.attributes, is_onedrive_base);
                            let entry = crate::domain::file_entry::FileEntry {
                                path: full_path,
                                name: dir_entry.name,
                                is_dir,
                                size: if is_dir { 0 } else { dir_entry.size },
                                modified: dir_entry.modified,
                                folder_cover: None,
                                drive_info: None,
                                sync_status,
                                deletion_date: None,
                                recycle_original_path: None,
                            };
                            all_entries_disk.push(entry.clone());
                            batch.push(entry);
                            if batch.len() >= batch_size {
                                let folders: Vec<PathBuf> = batch
                                    .iter()
                                    .filter(|e| e.is_dir)
                                    .map(|e| e.path.clone())
                                    .collect();
                                if !folders.is_empty() {
                                    let covers = disk_cache.get_folder_covers(&folders);
                                    for item in batch.iter_mut() {
                                        if item.is_dir {
                                            if let Some(cover) = covers.get(&item.path) {
                                                item.folder_cover = Some(cover.clone());
                                            }
                                        }
                                    }
                                }
                                let batch_len = batch.len();
                                let _ = file_entry_sender.send((my_gen, batch.clone()));
                                batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                                batch_size = batch_tracker.batch_size();
                                batch_start = std::time::Instant::now();
                                batch.clear();
                                ctx.request_repaint();
                            }
                        }
                    }
                    if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let folders: Vec<PathBuf> = batch
                            .iter()
                            .filter(|e| e.is_dir)
                            .map(|e| e.path.clone())
                            .collect();
                        if !folders.is_empty() {
                            let covers = disk_cache.get_folder_covers(&folders);
                            for item in batch.iter_mut() {
                                if item.is_dir {
                                    if let Some(cover) = covers.get(&item.path) {
                                        item.folder_cover = Some(cover.clone());
                                    }
                                }
                            }
                        }
                        let batch_len = batch.len();
                        let _ = file_entry_sender.send((my_gen, batch));
                        batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                        ctx.request_repaint();
                    }
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                    }
                    if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                        // Only cache for HDDs - SSDs bypass cache
                        if !is_ssd {
                            directory_cache
                                .put(PathBuf::from(&base_path), all_entries_disk.clone());
                        }
                        if let Some(di) = &directory_index_opt {
                            let indexed: Vec<IndexedFile> = all_entries_disk
                                .iter()
                                .map(|e| IndexedFile {
                                    name: e.name.clone(),
                                    size: e.size,
                                    modified: e.modified,
                                    is_dir: e.is_dir,
                                })
                                .collect();
                            let _ = di.put_directory(
                                &PathBuf::from(&base_path),
                                &indexed,
                                scan_start.elapsed().as_millis() as u64,
                            );
                        }
                    }
                    // DISABLED: Direct subdirectory prefetch (testing HDD I/O impact)
                    // if !is_ssd && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                    //     let subdirs: Vec<PathBuf> = all_entries_disk
                    //         .iter()
                    //         .filter(|e| e.is_dir)
                    //         .take(5)
                    //         .map(|e| e.path.clone())
                    //         .collect();
                    //     if !subdirs.is_empty() {
                    //         let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
                    //     }
                    // }
                    return;
                }
                // NTFS API returned None - filesystem may not be NTFS (e.g., exFAT)
                eprintln!(
                    "[FOLDER-LOADING] NTFS API returned None for {:?}, trying HDD-optimized path",
                    base_path
                );
            }

            // TIER 2: Try HDD-optimized FindFirstFileExW (for exFAT, FAT32, or when NTFS fails)
            if is_hdd && !used_optimized_path {
                match crate::infrastructure::windows::hdd_directory_reader::read_directory_hdd_batched(
                    &PathBuf::from(&base_path),
                    is_onedrive_base,
                ) {
                    Ok(batches) => {
                        eprintln!("[FOLDER-LOADING] TIER 2: Using HDD-optimized FindFirstFileExW for {:?}", base_path);
                        for batch_entries in batches {
                            if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                break;
                            }

                            // Process batch with folder covers
                            let folders: Vec<PathBuf> = batch_entries
                                .iter()
                                .filter(|e| e.is_dir)
                                .map(|e| e.path.clone())
                                .collect();

                            let mut processed_batch = batch_entries;
                            if !folders.is_empty() {
                                let covers = disk_cache.get_folder_covers(&folders);
                                for item in processed_batch.iter_mut() {
                                    if item.is_dir {
                                        if let Some(cover) = covers.get(&item.path) {
                                            item.folder_cover = Some(cover.clone());
                                        }
                                    }
                                }
                            }

                            all_entries_disk.extend(processed_batch.clone());
                            let batch_len = processed_batch.len();
                            let _ = file_entry_sender.send((my_gen, processed_batch));
                            batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                            batch_start = std::time::Instant::now();
                            ctx.request_repaint();
                        }

                        // Send empty batch to signal completion
                        if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                            let _ = file_entry_sender.send((my_gen, Vec::new()));
                            ctx.request_repaint();
                        }

                        // Cache results for future navigations
                        if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                            directory_cache.put(PathBuf::from(&base_path), all_entries_disk.clone());

                            if let Some(di) = &directory_index_opt {
                                let indexed: Vec<IndexedFile> = all_entries_disk
                                    .iter()
                                    .map(|e| IndexedFile {
                                        name: e.name.clone(),
                                        size: e.size,
                                        modified: e.modified,
                                        is_dir: e.is_dir,
                                    })
                                    .collect();
                                let _ = di.put_directory(
                                    &PathBuf::from(&base_path),
                                    &indexed,
                                    scan_start.elapsed().as_millis() as u64,
                                );
                            }
                        }

                        return;
                    }
                    Err(e) => {
                        eprintln!("[FOLDER-LOADING] TIER 2 failed: {}, falling back to standard Win32", e);
                        // Continue to TIER 3 (standard Win32)
                    }
                }
            }

            // TIER 3: Standard FindFirstFileW fallback (last resort)
            // CRITICAL FIX: For OneDrive folders, use timeout-protected enumeration
            // to prevent 30-60s blocking on folders with cloud-only files
            let is_onedrive = is_onedrive_base;

            if is_onedrive {
                // Use timeout-protected directory reading for OneDrive
                eprintln!("[FOLDER-LOADING] Using timeout-protected directory enumeration for OneDrive: {:?}", base_path);
                match onedrive::onedrive_read_directory(&PathBuf::from(&base_path)) {
                    onedrive::IoTimeoutResult::Ok(entries) => {
                        let mut batch: Vec<FileEntry> = Vec::with_capacity(entries.len().min(1000));
                        for (filename, attrs, size, modified) in entries {
                            if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                                break;
                            }

                            let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(
                                filename.to_lowercase().as_str(),
                                "desktop.ini"
                                    | "thumbs.db"
                                    | "$recycle.bin"
                                    | "system volume information"
                            );

                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.')
                            {
                                let mut is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
                                let full_path = PathBuf::from(&base_path).join(&filename);

                                if !is_dir && filename.to_lowercase().ends_with(".zip") {
                                    is_dir = true;
                                }

                                let is_zip = filename.to_lowercase().ends_with(".zip");
                                let file_size = if is_dir && !is_zip { 0 } else { size };

                                let sync_status = onedrive::get_sync_status(attrs, true);

                                let entry = FileEntry {
                                    path: full_path.clone(),
                                    name: filename.clone(),
                                    is_dir,
                                    size: file_size,
                                    modified,
                                    folder_cover: None,
                                    drive_info: None,
                                    sync_status,
                                    deletion_date: None,
                                    recycle_original_path: None,
                                };

                                batch.push(entry);

                                // Send batch when it reaches threshold
                                if batch.len() >= 1000 {
                                    let _batch_len = batch.len();
                                    let _ = file_entry_sender
                                        .send((my_gen, std::mem::take(&mut batch)));
                                    batch.clear();
                                    batch.reserve(1000);
                                }
                            }
                        }

                        // Send remaining entries
                        if !batch.is_empty() {
                            let _ = file_entry_sender.send((my_gen, batch));
                        }

                        // Signal completion
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();

                        eprintln!("[FOLDER-LOADING] OneDrive directory enumeration completed successfully");
                        return;
                    }
                    onedrive::IoTimeoutResult::Timeout => {
                        eprintln!("[FOLDER-LOADING] CRITICAL: OneDrive directory enumeration timed out after 5s for {:?}", base_path);
                        // CRITICAL FIX: Do NOT fall through to standard FindFirstFileW!
                        // Standard FindFirstFileW can block for 30-60s on OneDrive folders
                        // with cloud-only files, freezing the background thread and preventing
                        // any results from being sent to the UI.
                        // Send empty results to signal "could not load" gracefully.
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();
                        eprintln!(
                            "[FOLDER-LOADING] OneDrive enumeration timed out - sent empty results"
                        );
                        return;
                    }
                    onedrive::IoTimeoutResult::Err(_) => {
                        eprintln!("[FOLDER-LOADING] Error in OneDrive directory enumeration, falling back to standard");
                        // On error (not timeout), fall through to standard Win32 —
                        // errors are usually fast (milliseconds), not blocking.
                    }
                }
            }

            // Standard FindFirstFileW (for non-OneDrive or fallback)
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
                // `find_data` is a valid pointer to a `WIN32_FIND_DATAW` struct.
                // The handle returned is checked for validity and closed via `FindClose`
                // before the scope ends.
                if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
                    loop {
                        // Verifica se a geração mudou -> Aborta scan antigo
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
                            let full_path = PathBuf::from(&base_path).join(&filename);

                            // PERFORMANCE: Use basic attributes from FindFirstFileW/FindNextFileW.
                            // They already contain OneDrive flags (RECALL_ON_OPEN, RECALL_ON_DATA_ACCESS, PINNED).
                            // Calling GetFileAttributesW() again is redundant and adds 2ms per file!
                            //
                            // OLD CODE (removed - was causing 98% of scan time on OneDrive):
                            // let extended_attrs = if is_onedrive {
                            //     let path_wide: Vec<u16> = full_path.to_string_lossy()...
                            //     GetFileAttributesW(...)  // ← 2ms syscall PER FILE!
                            // } else {
                            //     attrs
                            // };
                            let extended_attrs = attrs;

                            // Filtros: hidden/system files
                            let is_hidden = (extended_attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (extended_attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(
                                filename.to_lowercase().as_str(),
                                "desktop.ini"
                                    | "thumbs.db"
                                    | "$recycle.bin"
                                    | "system volume information" // Re-adicionado "System Volume Information" para garantir compatibilidade
                            );

                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.')
                            {
                                let mut is_dir = (extended_attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;

                                // Treat ZIP files as navigable folders
                                if !is_dir && filename.to_lowercase().ends_with(".zip") {
                                    is_dir = true;
                                }

                                let is_zip = filename.to_lowercase().ends_with(".zip");
                                let size = if is_dir && !is_zip {
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

                                // OPTIMIZATION: Folder cover loading moved to batch (outside loop)
                                // to avoid N+1 queries in SQLite.
                                let folder_cover = None;

                                // Check if file is currently open (being used)
                                let sync_status =
                                    onedrive::get_sync_status(extended_attrs, is_onedrive);

                                // DISABLED: is_file_open() is EXTREMELY slow on OneDrive (28ms per file!)
                                // It tries to open file handles which triggers sync/network checks.
                                // Windows Explorer doesn't do this - it only uses file attributes.
                                //
                                // OLD CODE (removed for performance):
                                // if is_onedrive && !is_dir && sync_status != SyncStatus::None {
                                //     if onedrive::is_file_open(&full_path) {
                                //         sync_status = SyncStatus::Syncing;
                                //     }
                                // }

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover,
                                    drive_info: None,
                                    sync_status,
                                    deletion_date: None,
                                    recycle_original_path: None,
                                };

                                // Adiciona ao lote
                                all_entries_disk.push(entry.clone());
                                batch.push(entry);

                                // SE o lote encheu, envia e limpa (tamanho adaptado para SSD/HDD)
                                if batch.len() >= batch_size {
                                    // PRE-FETCH COVERS (Batch Optimization)
                                    let folders: Vec<PathBuf> = batch
                                        .iter()
                                        .filter(|e| e.is_dir)
                                        .map(|e| e.path.clone())
                                        .collect();

                                    if !folders.is_empty() {
                                        let covers = disk_cache.get_folder_covers(&folders);
                                        for item in batch.iter_mut() {
                                            if item.is_dir {
                                                if let Some(cover) = covers.get(&item.path) {
                                                    item.folder_cover = Some(cover.clone());
                                                }
                                            }
                                        }
                                    }

                                    let batch_len = batch.len();
                                    let _ = file_entry_sender.send((my_gen, batch.clone()));
                                    batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                                    batch_size = batch_tracker.batch_size();
                                    batch_start = std::time::Instant::now();
                                    batch.clear();
                                    ctx.request_repaint(); // Acorda a UI para mostrar progresso
                                }
                            }
                        }

                        if FindNextFileW(handle, &mut find_data).is_err() {
                            break;
                        }
                    }
                    let _ = FindClose(handle);
                }
            }

            // Envia o restante (último lote) se sobrou algo e a geração ainda é válida
            if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                // PRE-FETCH COVERS (Batch Optimization) - Last batch
                let folders: Vec<PathBuf> = batch
                    .iter()
                    .filter(|e| e.is_dir)
                    .map(|e| e.path.clone())
                    .collect();

                if !folders.is_empty() {
                    let covers = disk_cache.get_folder_covers(&folders);
                    for item in batch.iter_mut() {
                        if item.is_dir {
                            if let Some(cover) = covers.get(&item.path) {
                                item.folder_cover = Some(cover.clone());
                            }
                        }
                    }
                }

                let batch_len = batch.len();
                let _ = file_entry_sender.send((my_gen, batch));
                batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                ctx.request_repaint();
            }

            // Envia vetor VAZIO para sinalizar FIM do carregamento (apenas se a geração for a mesma)
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let scan_elapsed = scan_start.elapsed();
                eprintln!(
                    "[PERF] Folder scan complete: {:?} took {:.2}s",
                    current_path,
                    scan_elapsed.as_secs_f64()
                );
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }

            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                // CACHE STORAGE: Store in directory cache for instant future navigation (HDD ONLY)
                // SSDs bypass cache entirely - raw disk speed is faster than RAM cache
                if !is_ssd {
                    directory_cache.put(PathBuf::from(&base_path), all_entries_disk.clone());
                }

                if let Some(di) = &directory_index_opt {
                    let indexed: Vec<IndexedFile> = all_entries_disk
                        .iter()
                        .map(|e| IndexedFile {
                            name: e.name.clone(),
                            size: e.size,
                            modified: e.modified,
                            is_dir: e.is_dir,
                        })
                        .collect();
                    let _ = di.put_directory(
                        &PathBuf::from(&base_path),
                        &indexed,
                        scan_start.elapsed().as_millis() as u64,
                    );
                }
            }
            // DISABLED: Direct subdirectory prefetch (testing HDD I/O impact)
            // if !is_ssd && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
            //     let subdirs: Vec<PathBuf> = all_entries_disk
            //         .iter()
            //         .filter(|e| e.is_dir)
            //         .take(5)
            //         .map(|e| e.path.clone())
            //         .collect();
            //     if !subdirs.is_empty() {
            //         let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
            //     }
            // }
        });
    }
}
