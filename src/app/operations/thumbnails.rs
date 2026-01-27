//! Thumbnail loading requests
//!
//! This module handles requests for generating thumbnails and folder previews.

use std::path::PathBuf;
use crate::app::state::ImageViewerApp;
use crate::domain::thumbnail::ThumbnailData;
use crate::workers::thumbnail_worker::ThumbnailPriority;

impl ImageViewerApp {
    pub fn request_thumbnail_load(&mut self, path: PathBuf, size_px: u32) {
        // PERFORMANCE: Check RAM cache first before sending to worker
        // This avoids disk I/O entirely if the RGBA data is already in RAM
        if let Some((rgba_data, width, height)) = self.cache_manager.get_rgba_data(&path).map(|(d, w, h)| (d.clone(), *w, *h)) {
            // Data is in RAM cache - add directly to pending_thumbnails for GPU upload
            // No disk I/O needed!
            self.cache_manager.start_pending_upload(path.clone());
            self.pending_thumbnails.push_back(ThumbnailData {
                path,
                image_data: rgba_data,
                width,
                height,
                generation: self.generation,
            });
            return;
        }

        // Not in RAM cache - send to worker (will read from disk cache or generate)
        self.thumbnail_queue.push(path, self.generation, size_px, ThumbnailPriority::Interactive);
    }

    pub fn request_thumbnail_prefetch(&mut self, path: PathBuf, size_px: u32) {
        // PERFORMANCE: Check RAM cache first for prefetch too
        if let Some((rgba_data, width, height)) = self.cache_manager.get_rgba_data(&path).map(|(d, w, h)| (d.clone(), *w, *h)) {
            self.cache_manager.start_pending_upload(path.clone());
            self.pending_thumbnails.push_back(ThumbnailData {
                path,
                image_data: rgba_data,
                width,
                height,
                generation: self.generation,
            });
            return;
        }

        // Envia pedido BAIXA PRIORIDADE (Prefetch) com hint de tamanho
        self.thumbnail_queue.push(path, self.generation, size_px, ThumbnailPriority::Prefetch);
    }

    pub fn request_folder_preview_load(&mut self, path: PathBuf) {
        if self
            .cache_manager
            .start_folder_preview_loading(path.clone())
        {
            let _ = self.folder_preview_sender.send(path);
        }
    }
}
