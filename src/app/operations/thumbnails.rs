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
        self.cache_manager.thumbnail_trace.record_request(&path);

        let effective_size_px = self.effective_thumbnail_request_size_px(size_px);
        // Record the bucket we are about to request so the slot renderer
        // does NOT re-request this path once the worker returns. The slot
        // compares against this value, not the actual cached texture
        // dimensions (which can be smaller than the bucket for naturally
        // small source images and would otherwise loop forever).
        let attempted_bucket =
            crate::workers::thumbnail::processing::get_bucket_size(effective_size_px);
        self.cache_manager
            .note_attempted_thumbnail_bucket(&path, attempted_bucket);

        // When rendering the unfocused dual-panel pane, use the active generation
        // accepted by the shared thumbnail workers while preserving caller priority.
        let effective_gen = if self.use_active_generation_for_thumbnail_requests {
            let active_gen = self
                .current_generation
                .load(std::sync::atomic::Ordering::Relaxed);
            active_gen
        } else {
            self.generation
        };
        let effective_priority = priority;

        // Skip files pending deletion to avoid wasteful extraction
        if self
            .file_operation_state
            .pending_deletions
            .contains_key(&path)
        {
            self.cache_manager.thumbnail_trace.record_pending_deletion();
            return;
        }

        // Caller-side dedup signals — these reflect requests issued for paths
        // that are already in flight either at the worker side or pending GPU
        // upload. They should never be hot in idle.
        if self.cache_manager.loading_set.contains(&path) {
            self.cache_manager.thumbnail_trace.record_dup_loading();
        }
        if self.cache_manager.pending_upload_set.contains(&path) {
            self.cache_manager.thumbnail_trace.record_dup_pending();
        }

        // Per-path cooldown — defends against render-loop thrash when a freshly
        // uploaded thumbnail texture is somehow popped from the LRU between
        // frames (e.g. visible LRU rotation, generation-mismatch discards,
        // pending-queue evictions). Without this, the slot re-requests every
        // frame, the worker re-extracts, and each `ctx.load_texture` upload
        // accumulates GPU staging memory the OS releases more slowly than we
        // allocate it — producing a steady working-set leak even when the
        // texture cache cap is technically large enough to hold every visible
        // path. Mirrors the same fix applied to folder previews.
        if self.cache_manager.should_throttle_thumbnail_request(&path) {
            // Caller (file_slot/list_view) inserted into loading_set BEFORE
            // queueing the deferred action. Remove it so the next frame's
            // slot guard does not see is_loading=true forever.
            self.cache_manager.finish_loading(&path);
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
                self.cache_manager.thumbnail_trace.record_ram_cache_hit();
                // Data is in RAM cache - add directly to pending_thumbnails for GPU upload
                // No disk I/O needed!
                //
                // FIX: Remove from loading_set since we won't go through the worker channel.
                // The caller (file_slot/list_view) inserts into loading_set before calling us.
                // Without this, loading_set leaks entries on every RAM-cache hit, eventually
                // reaching the 200-entry cap and blocking ALL new thumbnail loads.
                self.cache_manager.finish_loading(&path);
                self.cache_manager.start_pending_upload(path.clone());
                self.cache_manager.note_thumbnail_request_sent(&path);
                self.pending_thumbnails.push_back(ThumbnailData {
                    path,
                    image_data: rgba_data,
                    width,
                    height,
                    generation: effective_gen,
                    priority: effective_priority,
                    not_found: false,
                });
                self.trim_pending_thumbnail_uploads_to_limit();
                return;
            }
        }

        // REMOVED: Lazy loading that prevented video thumbnail generation
        // Now all thumbnails are loaded, but with intelligent prioritization

        // Not in RAM cache - send to worker (will read from disk cache or generate)
        self.cache_manager.thumbnail_trace.record_worker_dispatch();
        self.cache_manager.note_thumbnail_request_sent(&path);
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
        self.cache_manager.folder_preview_trace.record_request();
        self.cache_manager
            .folder_preview_trace
            .record_request_path(&path);

        // Already in flight: dedup without poisoning the debounce timestamp.
        if self.cache_manager.is_folder_preview_loading(&path) {
            self.cache_manager
                .folder_preview_trace
                .record_duplicate_skip();
            return;
        }

        // Per-path cooldown — defends against render-loop thrash when the LRU
        // cap is smaller than the directory's folder set. Without this, an
        // evicted preview is re-requested every frame and each upload leaks
        // GPU staging memory.
        if self
            .cache_manager
            .should_throttle_folder_preview_request(&path)
        {
            self.cache_manager
                .folder_preview_trace
                .record_debounce_skip();
            return;
        }

        if !self
            .cache_manager
            .start_folder_preview_loading(path.clone())
        {
            // Loading-set rejection (full or duplicate). Don't poison the
            // cooldown — the renderer must be able to retry next frame.
            return;
        }

        let request = crate::workers::folder_preview_worker::FolderPreviewRequest {
            path: path.clone(),
            size_px: self.effective_folder_preview_request_size_px(),
        };
        match self.folder_preview_sender.try_send(request) {
            Ok(()) => {
                // Only NOW the request is committed to the worker pipeline —
                // record the cooldown to suppress redundant per-frame requests
                // until the upload completes (or the cooldown window expires).
                self.cache_manager.note_folder_preview_request_sent(&path);
            }
            Err(err) => {
                let request = err.into_inner();
                self.cache_manager
                    .finish_folder_preview_loading(&request.path);
                // Channel full — leave the cooldown untouched so the next
                // frame can retry as soon as a worker drains the queue.
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
