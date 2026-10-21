//! Detail-panel thumbnail recovery, independent of list/grid visibility.

use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::diagnostic_logger::{diag_info, field_u64};
use eframe::egui;
use std::path::PathBuf;

fn best_effort_texture_available(
    texture: Option<&egui::TextureHandle>,
    attempted_bucket: Option<u32>,
    required_bucket: u32,
) -> bool {
    // Bucket history survives LRU eviction; it is not proof a texture remains.
    texture.is_some() && attempted_bucket.is_some_and(|bucket| bucket >= required_bucket)
}

impl ImageViewerApp {
    pub fn ensure_detail_panel_thumbnail_for_file(&mut self, file: &FileEntry) {
        self.ensure_detail_panel_thumbnail_request(
            file.path.clone(),
            file.modified,
            file.is_media(),
            crate::domain::thumbnail::detail_preview_size(&file.path),
        );
    }

    pub(super) fn ensure_detail_panel_thumbnail_request(
        &mut self,
        path: PathBuf,
        modified: u64,
        is_media: bool,
        size: u32,
    ) {
        if !is_media || self.cache_manager.is_failed(&path) {
            return;
        }

        let effective_req_size = self.effective_thumbnail_request_size_px(size);
        let required_preview_bucket =
            crate::workers::thumbnail::processing::get_bucket_size(effective_req_size);
        let attempted_bucket = self.cache_manager.attempted_thumbnail_bucket_for(&path);
        let tex_in_cache = self.cache_manager.texture_cache.peek(&path);
        let has_required_texture = tex_in_cache.as_ref().is_some_and(|tex| {
            let tex_size = tex.size();
            (tex_size[0].max(tex_size[1]) as u32) >= size
        });
        let best_effort_available =
            best_effort_texture_available(tex_in_cache, attempted_bucket, required_preview_bucket);

        // Some sources cannot produce a thumbnail as large as requested. Accept
        // the cached result after the quality upgrade, without repeatedly loading it.
        let request_in_flight =
            self.cache_manager.is_loading(&path) || self.cache_manager.is_pending_upload(&path);
        let promote_best_effort = best_effort_available && !request_in_flight;

        if let Some(tex) = tex_in_cache {
            if self
                .selected_file
                .as_ref()
                .is_some_and(|selected| selected.path == path)
                && (has_required_texture || promote_best_effort)
            {
                self.selected_thumbnail = Some(tex.clone());
            }
        }

        if !has_required_texture {
            if request_in_flight {
                // Keep selected media ahead of list/grid prefetch requests.
                if self.cache_manager.is_loading(&path)
                    && !self.cache_manager.is_pending_upload(&path)
                {
                    let effective_gen = self.generation;
                    let request_epoch = self
                        .thumbnail_request_epochs
                        .get(&path)
                        .copied()
                        .unwrap_or(0);
                    if self.thumbnail_queue.promote_pending_to_interactive(
                        &path,
                        effective_gen,
                        effective_req_size,
                        0,
                        modified,
                        request_epoch,
                    ) {
                        self.cache_manager
                            .note_attempted_thumbnail_bucket(&path, required_preview_bucket);
                    }
                }
            } else if best_effort_available {
                if !self.cache_manager.best_effort_notified.contains(&path) {
                    self.cache_manager.best_effort_notified.insert(path.clone());
                    diag_info(
                        "preview_thumbnail",
                        "best_effort_accepted",
                        &[
                            field_u64("logical_req_size", size as u64),
                            field_u64("effective_req_size", effective_req_size as u64),
                            field_u64("attempted_bucket", attempted_bucket.unwrap_or(0) as u64),
                            field_u64("required_bucket", required_preview_bucket as u64),
                        ],
                    );
                }
            } else {
                self.cache_manager.loading_set.insert(path.clone());
                self.request_thumbnail_load_with_index_and_modified(path, size, 0, modified);
                // A quick tab round-trip can hit the request cooldown. Retry
                // after it expires even if the user stops moving the pointer.
                self.ui_ctx
                    .request_repaint_after(crate::ui::cache::THUMBNAIL_REQUEST_COOLDOWN);
            }
        }
    }
}

#[cfg(test)]
#[path = "detail_thumbnail_tests.rs"]
mod tests;
