use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::{is_image_extension, is_video_extension};
use std::path::PathBuf;

impl ImageViewerApp {
    /// Requests an async scan of a folder to discover the first image.
    /// OPTIMIZED: Sends message to a single worker (zero thread overhead)
    pub fn request_folder_scan(&mut self, folder_path: PathBuf) {
        self.request_folder_scans_batch(vec![folder_path]);
    }

    /// Batch version of request_folder_scan: resolves covers for multiple folders
    /// in a single SQLite query and calls filter_items() only once at the end.
    pub fn request_folder_scans_batch(&mut self, folder_paths: Vec<PathBuf>) {
        if folder_paths.is_empty() {
            return;
        }

        // 1. Single batched SQLite query for all folder covers
        let db_covers = self.disk_cache.get_folder_covers(&folder_paths);

        // 2. Resolve covers: DB hit → DirectoryIndex fallback → worker fallback
        let mut resolved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut worker_fallbacks: Vec<PathBuf> = Vec::new();

        for folder_path in folder_paths {
            let cover_opt = if let Some(cover) = db_covers.get(&folder_path) {
                Some(cover.clone())
            } else {
                // INDEX PATH: If DB has no cover, try DirectoryIndex (no HDD hit)
                let mut found = None;
                if let Some(di) = &self.directory_index {
                    if let Some((_meta, files)) = di.get_directory(&folder_path) {
                        for file in files.iter() {
                            if file.is_dir {
                                continue;
                            }
                            if let Some(ext) = std::path::Path::new(&file.name)
                                .extension()
                                .and_then(|e| e.to_str())
                            {
                                if is_image_extension(ext) || is_video_extension(ext) {
                                    found = Some(folder_path.join(&file.name));
                                    break;
                                }
                            }
                        }
                    }
                }
                found
            };

            if let Some(cover) = cover_opt {
                resolved.push((folder_path, cover));
            } else {
                worker_fallbacks.push(folder_path);
            }
        }

        // 3. Apply resolved covers to items (single pass through all_items)
        let mut any_updated = false;
        let mut thumb_loads: Vec<PathBuf> = Vec::new();
        if !resolved.is_empty() {
            // Build a lookup map for O(1) access
            let resolve_map: std::collections::HashMap<&PathBuf, &PathBuf> =
                resolved.iter().map(|(fp, cp)| (fp, cp)).collect();

            for item in self.all_items.iter_mut() {
                if let Some(cover) = resolve_map.get(&item.path) {
                    if item.folder_cover.as_ref() != Some(cover) {
                        item.folder_cover = Some((*cover).clone());
                        any_updated = true;
                    }
                    thumb_loads.push((*cover).clone());
                }
            }

            // Persist covers to DB outside the iter_mut loop
            for (folder_path, cover) in &resolved {
                self.disk_cache.set_folder_cover(folder_path, cover);
            }
        }

        // Request thumbnail loads (collected to avoid borrow conflicts)
        for cover in thumb_loads {
            if !self.cache_manager.has_thumbnail(&cover)
                && self.cache_manager.start_loading(cover.clone())
            {
                self.request_thumbnail_load(cover, 256);
            }
        }

        // 4. Single filter_items() call at the end
        if any_updated {
            self.filter_items();
            self.ui_ctx.request_repaint();
        }

        // 5. Send remaining folders to worker for HDD scan
        for path in worker_fallbacks {
            let _ = self.cover_worker_sender.send(path);
        }
    }
}
