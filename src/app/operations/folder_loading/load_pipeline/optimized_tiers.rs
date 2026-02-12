use crate::domain::file_entry::{is_archive_extension, FileEntry};
use crate::infrastructure::adaptive_batch::AdaptiveBatchTracker;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::{DirectoryIndex, IndexedFile};
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::ntfs_reader;
use crate::infrastructure::onedrive;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(super) fn try_handle_optimized_tiers(
    my_gen: usize,
    gen_clone: &Arc<AtomicUsize>,
    scan_start: &Instant,
    base_path: &str,
    is_ssd: bool,
    is_onedrive_base: bool,
    batch_size: &mut usize,
    batch_tracker: &mut AdaptiveBatchTracker,
    batch_start: &mut Instant,
    batch: &mut Vec<FileEntry>,
    all_entries_disk: &mut Vec<FileEntry>,
    file_entry_sender: &Sender<(usize, Vec<FileEntry>)>,
    ctx: &egui::Context,
    disk_cache: &Arc<ThumbnailDiskCache>,
    directory_cache: &Arc<DirectoryCache>,
    directory_index_opt: &Option<Arc<DirectoryIndex>>,
) -> bool {
    // OPTIMIZATION: Tiered disk reading strategy
    // Priority: 1) NTFS native API, 2) HDD-optimized FindFirstFileExW, 3) Standard FindFirstFileW
    let is_hdd = !is_ssd;
    let ntfs_api_available = ntfs_reader::is_available();

    // Track if we successfully used an optimized path
    let used_optimized_path = false;

    // TIER 1: Try NTFS native API first (fastest for NTFS drives)
    if is_hdd && ntfs_api_available {
        log::debug!(
            "[FOLDER-LOADING] TIER 1: Trying NTFS native API (NtQueryDirectoryFile) for {:?}",
            base_path
        );
        if let Some(entries) = ntfs_reader::read_directory_fast(&PathBuf::from(base_path)) {
            for dir_entry in entries {
                if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                    break;
                }
                let is_hidden = (dir_entry.attributes & 0x02) != 0;
                let is_system = (dir_entry.attributes & 0x04) != 0;
                let is_special = matches!(
                    dir_entry.name.to_lowercase().as_str(),
                    "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
                );
                if !is_hidden && !is_system && !is_special && !dir_entry.name.starts_with('.') {
                    let full_path = PathBuf::from(base_path).join(&dir_entry.name);
                    let mut is_dir = dir_entry.is_dir;
                    let is_archive = !is_dir && is_archive_extension(&dir_entry.name);
                    if is_archive {
                        is_dir = true;
                    }
                    let sync_status =
                        onedrive::get_sync_status(dir_entry.attributes, is_onedrive_base);
                    let entry = crate::domain::file_entry::FileEntry {
                        path: full_path,
                        name: dir_entry.name,
                        is_dir,
                        size: if is_dir && !is_archive {
                            0
                        } else {
                            dir_entry.size
                        },
                        modified: dir_entry.modified,
                        folder_cover: None,
                        drive_info: None,
                        sync_status,
                        deletion_date: None,
                        recycle_original_path: None,
                    };
                    all_entries_disk.push(entry.clone());
                    batch.push(entry);
                    if batch.len() >= *batch_size {
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
                        *batch_size = batch_tracker.batch_size();
                        *batch_start = std::time::Instant::now();
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
                let _ = file_entry_sender.send((my_gen, std::mem::take(batch)));
                batch_tracker.record_batch(batch_start.elapsed(), batch_len);
                ctx.request_repaint();
            }
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                directory_cache.put(PathBuf::from(base_path), all_entries_disk.clone());
                if let Some(di) = directory_index_opt {
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
                        &PathBuf::from(base_path),
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
            return true;
        }
        // NTFS API returned None - filesystem may not be NTFS (e.g., exFAT)
        log::debug!(
            "[FOLDER-LOADING] NTFS API returned None for {:?}, trying HDD-optimized path",
            base_path
        );
    }

    // TIER 2: Try HDD-optimized FindFirstFileExW (for exFAT, FAT32, or when NTFS fails)
    if is_hdd && !used_optimized_path {
        match crate::infrastructure::windows::hdd_directory_reader::read_directory_hdd_batched(
            &PathBuf::from(base_path),
            is_onedrive_base,
        ) {
            Ok(batches) => {
                log::debug!(
                    "[FOLDER-LOADING] TIER 2: Using HDD-optimized FindFirstFileExW for {:?}",
                    base_path
                );
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
                    *batch_start = std::time::Instant::now();
                    ctx.request_repaint();
                }

                // Send empty batch to signal completion
                if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                    let _ = file_entry_sender.send((my_gen, Vec::new()));
                    ctx.request_repaint();
                }

                // Cache results for future navigations
                if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                    directory_cache.put(PathBuf::from(base_path), all_entries_disk.clone());

                    if let Some(di) = directory_index_opt {
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
                            &PathBuf::from(base_path),
                            &indexed,
                            scan_start.elapsed().as_millis() as u64,
                        );
                    }
                }

                return true;
            }
            Err(e) => {
                log::warn!(
                    "[FOLDER-LOADING] TIER 2 failed: {}, falling back to standard Win32",
                    e
                );
                // Continue to TIER 3 (standard Win32)
            }
        }
    }

    false
}
