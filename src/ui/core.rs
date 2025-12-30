//! Core application logic for the file manager.
//!
//! This module contains the main business logic for folder loading,
//! sorting, filtering, and message processing.

use std::cmp::Ordering;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::Ordering as AtomicOrdering;

use eframe::egui;

use crate::domain::file_entry::{FileEntry, SortMode};
use crate::ui::app::ImageViewerApp;

// Windows API imports for file scanning
use windows::{
    core::*,
    Win32::Storage::FileSystem::*,
};

impl ImageViewerApp {
    /// Loads the current folder asynchronously.
    pub fn load_folder(&mut self) {
        self.generation += 1; // Increment local generation
        self.current_generation.store(self.generation, AtomicOrdering::Relaxed); // Sync with workers
        
        // 1. State Cleanup (UI Thread)
        self.items.clear();
        self.all_items.clear();  // Clear master backup too
        self.texture_cache.clear();
        self.loading_set.clear();
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.total_items = 0;
        
        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        
        // STREAMING BATCH LOADING: Sends batches of 250 items progressively
        std::thread::spawn(move || {
            // Buffer for batch sending
            let mut batch = Vec::with_capacity(250);
            
            // Prepare Win32 search
            let search_path = if current_path.ends_with('\\') {
                format!("{}*", current_path)
            } else {
                format!("{}\\*", current_path)
            };
            let wide_path: Vec<u16> = search_path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut find_data = WIN32_FIND_DATAW::default();

            unsafe {
                if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
                    loop {
                        // Check if generation changed -> Abort old scan
                        if gen_clone.load(AtomicOrdering::Relaxed) != my_gen { break; }

                        let len = find_data.cFileName.iter().position(|&c| c == 0).unwrap_or(find_data.cFileName.len());
                        let filename = std::ffi::OsString::from_wide(&find_data.cFileName[0..len])
                            .to_string_lossy()
                            .into_owned();

                        if filename != "." && filename != ".." {
                            let attrs = find_data.dwFileAttributes;
                            
                            // Filters: hidden/system files
                            let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(filename.to_lowercase().as_str(),
                                "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
                                // Re-added "System Volume Information" for compatibility
                            );
                            
                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.') {
                                let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
                                let full_path = PathBuf::from(&current_path).join(&filename);

                                let size = if is_dir { 
                                    0 
                                } else {
                                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
                                };

                                let ft = find_data.ftLastWriteTime;
                                let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                                let modified = if windows_ticks > 116444736000000000 {
                                    (windows_ticks - 116444736000000000) / 10_000_000
                                } else {
                                    0
                                };

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover: None,  // Lazy load
                                };

                                // Add to batch
                                batch.push(entry);

                                // IF batch is full (250 items), send and clear
                                if batch.len() >= 250 {
                                    let _ = file_entry_sender.send((my_gen, batch.clone()));
                                    batch.clear();
                                    ctx.request_repaint(); // Wake UI to show progress
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

            // Send remainder (last batch) if something left and generation is still valid
            if !batch.is_empty() && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, batch));
                ctx.request_repaint();
            }
            
            // Send EMPTY vector to signal END of loading (only if generation is same)
            if gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
            }
        });
    }
    
    /// Filters items based on search query.
    pub fn filter_items(&mut self) {
        if self.search_query.is_empty() {
            self.items = self.all_items.clone();
        } else {
            let query = self.search_query.to_lowercase();
            self.items = self.all_items.iter()
                .filter(|item| item.name.to_lowercase().contains(&query))
                .cloned()
                .collect();
        }
        self.total_items = self.items.len();
    }
    
    /// Sorts items based on current mode (keeps folders always first).
    pub fn sort_items(&mut self) {
        self.items.sort_by(|a, b| {
            // 1. Folders always first (unless both are folders or both files)
            if a.is_dir != b.is_dir {
                return if a.is_dir {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            
            // 2. Sort by selected mode
            let ordering = match self.sort_mode {
                SortMode::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortMode::Date => a.modified.cmp(&b.modified),
                SortMode::Size => a.size.cmp(&b.size),
            };
            
            // 3. Reverse if descending is active
            if self.sort_descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    
    /// Requests async scan of a folder to discover first image.
    /// OPTIMIZED: Sends message to single worker (zero thread overhead).
    pub fn request_folder_scan(&self, folder_path: PathBuf) {
        // Just sends to queue - worker processes in background
        let _ = self.cover_worker_sender.send(folder_path);
    }
    
    /// Requests thumbnail load for a path.
    pub fn request_thumbnail_load(&self, path: PathBuf) {
        // Send request to Worker Pool with current generation
        let _ = self.thumbnail_req_sender.send((path, self.generation));
    }
    
    /// Processes incoming messages from worker channels.
    pub fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        // 1. CHECK MANUAL REFRESH (F5)
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.load_folder();
        }

        // 2. CHECK AUTO-REFRESH (WATCHER)
        while let Ok(event) = self.fs_event_receiver.try_recv() {
            match event {
                Ok(_) => self.pending_auto_reload = true,
                Err(e) => eprintln!("Watch error: {:?}", e),
            }
        }

        // Execute reload only when debounce allows
        if self.pending_auto_reload {
            let elapsed = self.last_auto_reload.elapsed();
            if elapsed > std::time::Duration::from_millis(500) {
                // VALIDATE IF CURRENT PATH STILL EXISTS (may have been renamed/deleted)
                if std::path::Path::new(&self.current_path).exists() {
                    self.load_folder();
                } else {
                    self.go_up_one_level();
                }
                self.last_auto_reload = std::time::Instant::now();
                self.pending_auto_reload = false;
            }
        }

        // 1. STREAMING: Receives incremental batches of FileEntry (Filtered by generation)
        while let Ok((gen_id, new_batch)) = self.file_entry_receiver.try_recv() {
            if gen_id != self.generation { 
                continue; // Discard data from previous navigation/refresh
            }

            if new_batch.is_empty() {
                // Empty batch = "End of Loading" signal from thread
                self.is_loading_folder = false;
                // Final sorting to ensure everything correct
                self.sort_items();
            } else {
                // Data arrived! Add to master list
                self.all_items.extend(new_batch);
                
                // Reapply filter and sorting incrementally
                self.filter_items(); 
                self.sort_items();
            }
            ctx.request_repaint();
        }
        
        // 2. Cover Worker: Receives folder cover results
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Update in items (filtered/sorted list)
                if let Some(item) = self.items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    
                    // Already request thumbnail of found image
                    if !self.texture_cache.contains(&cover) && !self.loading_set.contains(&cover) {
                        self.request_thumbnail_load(cover.clone());
                    }
                    folder_updates = true;
                }
                // Also update in all_items (persistence through filters)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover);
                }
            }
        }
        if folder_updates {
            ctx.request_repaint();
        }
        
        // 3. Individual thumbnails
        let mut received_any = false;
        
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            // --- MEMORY VALIDATION ---
            // If image belongs to previous generation (another folder), discard.
            if thumbnail_data.generation != self.generation as usize {
                continue;
            }
            // ----------------------------

            received_any = true;
            
            // Only process thumbnails (image_data not empty)
            if !thumbnail_data.image_data.is_empty() {
                self.loading_set.remove(&thumbnail_data.path);
                
                let texture = ctx.load_texture(
                    thumbnail_data.path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [thumbnail_data.width as usize, thumbnail_data.height as usize],
                        &thumbnail_data.image_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                
                self.texture_cache.put(thumbnail_data.path, texture);
            }
        }

        
        if received_any {
            ctx.request_repaint();
        }
    }
}
