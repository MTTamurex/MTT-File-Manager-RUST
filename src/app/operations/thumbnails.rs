//! Thumbnail loading requests
//!
//! This module handles requests for generating thumbnails and folder previews.

use crate::app::state::ImageViewerApp;
use crate::domain::thumbnail::ThumbnailData;
use crate::workers::thumbnail::ThumbnailPriority;
use std::path::PathBuf;

impl ImageViewerApp {
    pub fn request_thumbnail_load(&mut self, path: PathBuf, size_px: u32) {
        self.request_thumbnail_load_internal(
            path,
            size_px,
            None,
            ThumbnailPriority::Interactive,
            0,
        );
    }

    pub fn request_thumbnail_load_with_modified(
        &mut self,
        path: PathBuf,
        size_px: u32,
        modified: u64,
    ) {
        self.request_thumbnail_load_internal(
            path,
            size_px,
            None,
            ThumbnailPriority::Interactive,
            modified,
        );
    }

    pub fn request_thumbnail_load_with_index(
        &mut self,
        path: PathBuf,
        size_px: u32,
        directory_index: usize,
    ) {
        self.request_thumbnail_load_internal(
            path,
            size_px,
            Some(directory_index),
            ThumbnailPriority::Interactive,
            0,
        );
    }

    pub fn request_thumbnail_load_with_index_and_modified(
        &mut self,
        path: PathBuf,
        size_px: u32,
        directory_index: usize,
        modified: u64,
    ) {
        self.request_thumbnail_load_internal(
            path,
            size_px,
            Some(directory_index),
            ThumbnailPriority::Interactive,
            modified,
        );
    }

    pub fn request_thumbnail_prefetch(&mut self, path: PathBuf, size_px: u32) {
        self.request_thumbnail_load_internal(path, size_px, None, ThumbnailPriority::Prefetch, 0);
    }

    pub fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size_px: u32,
        directory_index: usize,
    ) {
        self.request_thumbnail_load_internal(
            path,
            size_px,
            Some(directory_index),
            ThumbnailPriority::Prefetch,
            0,
        );
    }

    pub fn request_thumbnail_prefetch_with_index_and_modified(
        &mut self,
        path: PathBuf,
        size_px: u32,
        directory_index: usize,
        modified: u64,
    ) {
        self.request_thumbnail_load_internal(
            path,
            size_px,
            Some(directory_index),
            ThumbnailPriority::Prefetch,
            modified,
        );
    }

    fn request_thumbnail_load_internal(
        &mut self,
        path: PathBuf,
        size_px: u32,
        directory_index: Option<usize>,
        priority: ThumbnailPriority,
        modified: u64,
    ) {
        // Skip files pending deletion to avoid wasteful extraction
        if self.pending_deletions.contains_key(&path) {
            return;
        }

        // PERFORMANCE: Check RAM cache first before sending to worker
        // This avoids disk I/O entirely if the RGBA data is already in RAM
        if let Some((rgba_data, width, height)) = self
            .cache_manager
            .get_rgba_data(&path)
            .map(|(d, w, h)| (d.clone(), *w, *h))
        {
            let cached_max_dim = width.max(height);

            // Only reuse RAM cache if it meets or exceeds the requested size
            if cached_max_dim >= size_px {
                // Data is in RAM cache - add directly to pending_thumbnails for GPU upload
                // No disk I/O needed!
                self.cache_manager.start_pending_upload(path.clone());
                self.pending_thumbnails.push_back(ThumbnailData {
                    path,
                    image_data: rgba_data,
                    width,
                    height,
                    generation: self.generation,
                    not_found: false,
                });
                return;
            }
        }

        // REMOVED: Lazy loading that prevented video thumbnail generation
        // Now all thumbnails are loaded, but with intelligent prioritization

        // Not in RAM cache - send to worker (will read from disk cache or generate)
        if let Some(index) = directory_index {
            self.thumbnail_queue.push_with_index(
                path,
                self.generation,
                size_px,
                priority,
                Some(index),
                modified,
            );
        } else {
            self.thumbnail_queue
                .push(path, self.generation, size_px, priority, modified);
        }
    }

    pub fn request_folder_preview_load(&mut self, path: PathBuf) {
        if self
            .cache_manager
            .start_folder_preview_loading(path.clone())
        {
            let _ = self.folder_preview_sender.send(path);
        }
    }

    pub fn request_icon_load(&mut self, path: PathBuf) {
        if !self.loading_icons.contains(&path) {
            self.loading_icons.insert(path.clone());
            let _ = self.icon_req_sender.send(path);
        }
    }
}

#[cfg(test)]
mod tests {}
