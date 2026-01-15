//! Thumbnail loading requests
//!
//! This module handles requests for generating thumbnails and folder previews.

use std::path::PathBuf;
use crate::app::state::ImageViewerApp;
use crate::workers::thumbnail_worker::ThumbnailPriority;

impl ImageViewerApp {
    pub fn request_thumbnail_load(&self, path: PathBuf, size_px: u32) {
        // Envia pedido PRIORITÁRIO (Visível) com hint de tamanho
        self.thumbnail_queue.push(path, self.generation, size_px, ThumbnailPriority::High);
    }

    pub fn request_thumbnail_prefetch(&self, path: PathBuf, size_px: u32) {
        // Envia pedido BAIXA PRIORIDADE (Prefetch) com hint de tamanho
        self.thumbnail_queue.push(path, self.generation, size_px, ThumbnailPriority::Low);
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
