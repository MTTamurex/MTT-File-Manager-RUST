//! Cache-only workers never inspect the source drive. Slow misses and freshness
//! checks use a separate bounded queue, so Cryptomator cannot stall cache hits.
use super::{FolderPreviewData, FolderPreviewRequest, FolderPreviewRequestMode};
use crate::domain::folder_appearance::FolderAppearance;
use crate::infrastructure::disk_cache::{FolderPreviewSource, ThumbnailDiskCache};
use crate::workers::thumbnail::processing::get_bucket_size;
use eframe::egui;
use std::sync::{mpsc::Sender, Arc};

fn cached_result(
    request: &mut FolderPreviewRequest,
    appearance: FolderAppearance,
    disk_cache: &ThumbnailDiskCache,
) -> Option<FolderPreviewData> {
    if request.mode != FolderPreviewRequestMode::Load || !request.validity.is_current() {
        return None;
    }
    let bucket = get_bucket_size(request.size_px);
    let mut result = request.empty_result(appearance);
    if let Some(cached) = request.cached_preview.take().filter(|cached| {
        cached.matches_request(
            &request.path,
            request.cover_path.as_deref(),
            bucket,
            appearance,
        )
    }) {
        result.cache_source = Some(cached.source);
        result.rgba_data = cached.pixels;
        result.width = cached.width;
        result.height = cached.height;
    } else {
        let snapshot = disk_cache.get_folder_preview_snapshot(&request.path, bucket, appearance)?;
        result.cache_source = snapshot.fingerprint.map(|fingerprint| {
            FolderPreviewSource::from_cached_fingerprint(
                &request.path,
                request.cover_path.as_deref(),
                fingerprint,
            )
        });
        let mut pixels = snapshot.pixels;
        crate::domain::thumbnail::premultiply_rgba_in_place(&mut pixels);
        result.rgba_data = Arc::new(pixels);
        result.width = snapshot.width;
        result.height = snapshot.height;
    }
    result.validation_pending = true;
    request.validation_source = result.cache_source.clone();
    request.mode = FolderPreviewRequestMode::ValidateCached;
    Some(result)
}

pub fn spawn_folder_preview_cache_worker(
    requests: crossbeam_channel::Receiver<FolderPreviewRequest>,
    slow_requests: crossbeam_channel::Sender<FolderPreviewRequest>,
    results: Sender<FolderPreviewData>,
    ctx: egui::Context,
    disk_cache: Arc<ThumbnailDiskCache>,
) {
    if let Err(error) = std::thread::Builder::new()
        .name("folder-preview-cache".into())
        .spawn(move || {
            while let Ok(mut request) = requests.recv() {
                if !request.validity.is_current() {
                    continue;
                }
                let appearance = crate::ui::folder_appearance::current(&ctx);
                if let Some(data) = cached_result(&mut request, appearance, &disk_cache) {
                    // Send before dispatching validation: terminal/corrected results
                    // must never overtake the speculative image in the UI channel.
                    if results.send(data).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
                // A saturated source queue must not block other cache hits.
                if let Err(error) = slow_requests.try_send(request) {
                    let request = error.into_inner();
                    let mut completion = request.empty_result(appearance);
                    completion.replace_existing =
                        request.mode == FolderPreviewRequestMode::ValidateCached;
                    completion.retry_after_source_change =
                        request.mode == FolderPreviewRequestMode::ValidateCached;
                    if results.send(completion).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                }
            }
        })
    {
        log::error!("[FolderPreview] Failed to start cache worker: {error}");
    }
}

#[cfg(test)]
mod tests;
