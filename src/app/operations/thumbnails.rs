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
        let effective_size_px = self.effective_thumbnail_request_size_px(size_px);

        // When rendering the inactive dual-panel, use the active panel's generation
        // (current_generation always holds the active panel's gen, since both panels
        // share the same Arc<AtomicUsize>) and downgrade to Prefetch priority so the
        // active panel's Interactive requests are processed first.
        //
        // Previously this code suppressed all inactive-panel requests, which prevented
        // their thumbnails from ever being loaded into the shared texture_cache.
        // Now we allow the requests through with the correct generation so the worker
        // accepts them, results flow into the shared cache, and both panels display
        // thumbnails independently.
        let (effective_gen, effective_priority) = if self.suppress_thumbnail_requests {
            let active_gen = self
                .current_generation
                .load(std::sync::atomic::Ordering::Relaxed);
            (active_gen, ThumbnailPriority::Prefetch)
        } else {
            (self.generation, priority)
        };

        // Skip files pending deletion to avoid wasteful extraction
        if self
            .file_operation_state
            .pending_deletions
            .contains_key(&path)
        {
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
            if cached_max_dim >= effective_size_px {
                // Data is in RAM cache - add directly to pending_thumbnails for GPU upload
                // No disk I/O needed!
                //
                // FIX: Remove from loading_set since we won't go through the worker channel.
                // The caller (file_slot/list_view) inserts into loading_set before calling us.
                // Without this, loading_set leaks entries on every RAM-cache hit, eventually
                // reaching the 200-entry cap and blocking ALL new thumbnail loads.
                self.cache_manager.finish_loading(&path);
                self.cache_manager.start_pending_upload(path.clone());
                self.pending_thumbnails.push_back(ThumbnailData {
                    path,
                    image_data: rgba_data,
                    width,
                    height,
                    generation: effective_gen,
                    not_found: false,
                });
                self.trim_pending_thumbnail_uploads_to_limit();
                return;
            }
        }

        // REMOVED: Lazy loading that prevented video thumbnail generation
        // Now all thumbnails are loaded, but with intelligent prioritization

        // Not in RAM cache - send to worker (will read from disk cache or generate)
        if let Some(index) = directory_index {
            self.thumbnail_queue.push_with_index(
                path,
                effective_gen,
                effective_size_px,
                effective_priority,
                Some(index),
                modified,
            );
        } else {
            self.thumbnail_queue.push(
                path,
                effective_gen,
                effective_size_px,
                effective_priority,
                modified,
            );
        }
    }

    pub fn request_folder_preview_load(&mut self, path: PathBuf) {
        if self
            .cache_manager
            .start_folder_preview_loading(path.clone())
        {
            let request = crate::workers::folder_preview_worker::FolderPreviewRequest {
                path,
                size_px: self.effective_folder_preview_request_size_px(),
            };
            if let Err(err) = self.folder_preview_sender.try_send(request) {
                let request = err.into_inner();
                self.cache_manager
                    .finish_folder_preview_loading(&request.path);
            }
        }
    }

    pub fn request_icon_load(&mut self, path: PathBuf) {
        if self.loading_icons.contains(&path) {
            return;
        }

        // Dedup by extension: if another file with the same extension is already
        // being loaded, skip.  Once that result arrives, extension_cache is
        // populated and ALL files with that extension get the icon immediately.
        // Only applies to non-unique-icon extensions.
        if let Some(ext) = path.extension() {
            let ext_lower = ext.to_string_lossy().to_lowercase();
            if !crate::infrastructure::windows::icons::is_per_file_icon_ext(&ext_lower) {
                let load_ext =
                    crate::infrastructure::windows::icons::canonical_icon_ext(&ext_lower);
                if self.loading_extensions.contains(load_ext) {
                    return; // Another file with this ext is already in-flight.
                }
                self.loading_extensions.insert(load_ext.to_string());
            }
        }

        self.loading_icons.insert(path.clone());
        if self
            .icon_req_sender
            .send((path.clone(), self.generation))
            .is_err()
        {
            self.loading_icons.remove(&path);
        }
    }
}

#[cfg(test)]
mod tests {}
