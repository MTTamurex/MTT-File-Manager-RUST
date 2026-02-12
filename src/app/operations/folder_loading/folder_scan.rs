use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::{is_image_extension, is_video_extension};
use std::path::PathBuf;

impl ImageViewerApp {
    /// Requests an async scan of a folder to discover the first image.
    /// OPTIMIZED: Sends message to a single worker (zero thread overhead)
    pub fn request_folder_scan(&mut self, folder_path: PathBuf) {
        // FAST PATH: Check folder cover in DB (no HDD hit)
        let mut cover_opt = self
            .disk_cache
            .get_folder_covers(std::slice::from_ref(&folder_path))
            .get(&folder_path)
            .cloned();

        // INDEX PATH: If DB has no cover, try DirectoryIndex (no HDD hit)
        if cover_opt.is_none() {
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
                                cover_opt = Some(folder_path.join(&file.name));
                                break;
                            }
                        }
                    }
                }
            }
        }

        if let Some(cover) = cover_opt {
            // Persist cover to DB (NVMe) so we don't hit HDD next time
            self.disk_cache.set_folder_cover(&folder_path, &cover);

            let mut updated = false;
            if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                if item.folder_cover.as_ref() != Some(&cover) {
                    item.folder_cover = Some(cover.clone());
                    updated = true;
                }
            }

            if !self.cache_manager.has_thumbnail(&cover)
                && self.cache_manager.start_loading(cover.clone())
            {
                self.request_thumbnail_load(cover, 256);
            }

            if updated {
                self.filter_items();
                self.ui_ctx.request_repaint();
            }
            return;
        }

        // Fallback: send to worker (will scan HDD)
        let _ = self.cover_worker_sender.send(folder_path);
    }
}
