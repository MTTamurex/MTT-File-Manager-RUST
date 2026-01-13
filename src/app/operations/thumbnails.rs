//! Thumbnail loading requests
//!
//! This module handles requests for generating thumbnails and folder previews.

use std::path::PathBuf;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn request_thumbnail_load(&self, path: PathBuf) {
        // Envia pedido para o Worker Pool com a geração atual
        let _ = self.thumbnail_req_sender.send((path, self.generation));
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
