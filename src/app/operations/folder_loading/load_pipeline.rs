use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::{is_archive_extension, FileEntry};
use crate::infrastructure::adaptive_batch::{AdaptiveBatchConfig, AdaptiveBatchTracker};
use crate::infrastructure::directory_index::IndexedFile;
use crate::infrastructure::io_priority;
use crate::infrastructure::onedrive;
mod fast_paths;
mod optimized_tiers;

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

            // STALE-WHILE-REVALIDATE STRATEGY: Instant feedback via DirectoryCache
            let base_path_buf = PathBuf::from(&base_path);
            // PERFORMANCE: Only use is_onedrive_path() which is string-based (no I/O)
            // path_has_cloud_attributes() was removed because GetFileAttributesW can BLOCK
            // indefinitely on cloud-only OneDrive folders
            let is_onedrive_base = onedrive::is_onedrive_path(&base_path_buf);
            let mut batch = Vec::with_capacity(batch_size);
            let mut all_entries_disk: Vec<FileEntry> = Vec::new();
            let mut batch_start = std::time::Instant::now();
            if fast_paths::try_handle_fast_paths(
                my_gen,
                &gen_clone,
                &current_path,
                force_refresh,
                &base_path,
                &base_path_buf,
                is_ssd,
                is_onedrive_base,
                &mut batch_size,
                &mut batch_tracker,
                &mut batch_start,
                &file_entry_sender,
                &ctx,
                &disk_cache,
                &directory_cache,
                &directory_index_opt,
            ) {
                return;
            }

            if optimized_tiers::try_handle_optimized_tiers(
                my_gen,
                &gen_clone,
                &scan_start,
                &base_path,
                is_ssd,
                is_onedrive_base,
                &mut batch_size,
                &mut batch_tracker,
                &mut batch_start,
                &mut batch,
                &mut all_entries_disk,
                &file_entry_sender,
                &ctx,
                &disk_cache,
                &directory_cache,
                &directory_index_opt,
            ) {
                return;
            }

            // TIER 3: Standard FindFirstFileW fallback (last resort)
            // CRITICAL FIX: For OneDrive folders, use timeout-protected enumeration
            // to prevent 30-60s blocking on folders with cloud-only files
            let is_onedrive = is_onedrive_base;

            if is_onedrive {
                // Use timeout-protected directory reading for OneDrive
                eprintln!(
                    "[FOLDER-LOADING] Using timeout-protected directory enumeration for OneDrive: {:?}",
                    base_path
                );
                let onedrive_enum_start = std::time::Instant::now();
                match onedrive::onedrive_read_directory(&PathBuf::from(&base_path)) {
                    onedrive::IoTimeoutResult::Ok(entries) => {
                        eprintln!(
                            "[PERF] OneDrive enum complete: {:?} items={} elapsed={}ms",
                            base_path,
                            entries.len(),
                            onedrive_enum_start.elapsed().as_millis()
                        );
                        batch.clear();
                        batch.reserve(batch_size.max(64));

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

                                let is_archive = is_archive_extension(&filename);
                                if !is_dir && is_archive {
                                    is_dir = true;
                                }

                                let file_size = if is_dir && !is_archive { 0 } else { size };

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

                                all_entries_disk.push(entry.clone());
                                batch.push(entry);

                                // Send adaptive batches to improve first paint.
                                if batch.len() >= batch_size {
                                    let batch_len = batch.len();
                                    let _ = file_entry_sender
                                        .send((my_gen, std::mem::take(&mut batch)));
                                    batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                                    batch_size = batch_tracker.batch_size();
                                    batch_start = std::time::Instant::now();
                                    batch.reserve(batch_size.max(64));
                                    ctx.request_repaint();
                                }
                            }
                        }

                        // Send remaining entries
                        if !batch.is_empty() {
                            let batch_len = batch.len();
                            let _ = file_entry_sender.send((my_gen, std::mem::take(&mut batch)));
                            batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                        }

                        // Signal completion
                        let _ = file_entry_sender.send((my_gen, Vec::new()));
                        ctx.request_repaint();

                        // Populate caches so subsequent OneDrive navigations are instant.
                        if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                            directory_cache
                                .put(PathBuf::from(&base_path), all_entries_disk.clone());

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

                        eprintln!(
                            "[FOLDER-LOADING] OneDrive directory enumeration completed successfully in {}ms (visible_items={})",
                            onedrive_enum_start.elapsed().as_millis(),
                            all_entries_disk.len()
                        );
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

                                // Treat archive files as navigable folders
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
                // CACHE STORAGE: store for instant future navigation (all local disks)
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
