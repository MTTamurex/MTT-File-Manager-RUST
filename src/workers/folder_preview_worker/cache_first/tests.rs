use super::*;
use crate::domain::folder_preview_validity::FolderPreviewRequests;
use crate::workers::folder_preview_worker::CachedFolderPreview;
use std::path::PathBuf;
use std::time::Duration;

fn cache_at(dir: &tempfile::TempDir) -> Arc<ThumbnailDiskCache> {
    Arc::new(ThumbnailDiskCache::new(dir.path().join("cache")).unwrap())
}

fn put_snapshot(cache: &ThumbnailDiskCache, path: &std::path::Path) {
    // Source deliberately does not exist. Only SQLite may be used to display it.
    cache.put_folder_preview_cache(path, 128, &[40, 80, 120, 128].repeat(128 * 128), 128, 128);
}

#[test]
fn folder_preview_cache_first_displays_db_artwork_without_source_access() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_at(&dir);
    let path = dir.path().join("unavailable-folder");
    put_snapshot(&cache, &path);
    let mut request = FolderPreviewRequest::new(path, 64, None);
    let result = cached_result(&mut request, FolderAppearance::Original, &cache).unwrap();
    assert_eq!((result.width, result.height), (128, 128));
    assert_eq!(result.rgba_data.len(), 128 * 128 * 4);
    assert!(result.premultiplied && result.validation_pending);
    assert!(!result.replace_existing);
    assert_eq!(request.mode, FolderPreviewRequestMode::ValidateCached);
}

#[test]
fn folder_preview_cache_first_preserves_ram_source_for_background_validation() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_at(&dir);
    let path = dir.path().join("folder");
    std::fs::create_dir(&path).unwrap();
    let source = FolderPreviewSource::capture(&path, None).unwrap();
    let pixels = Arc::new(vec![255; 128 * 128 * 4]);
    let cached = CachedFolderPreview {
        pixels: pixels.clone(),
        width: 128,
        height: 128,
        appearance: FolderAppearance::Original,
        source: source.clone(),
    };
    std::fs::rename(&path, dir.path().join("renamed-folder")).unwrap();
    let mut request = FolderPreviewRequest::new(path, 64, None).with_cached_preview(Some(cached));
    let result = cached_result(&mut request, FolderAppearance::Original, &cache).unwrap();
    assert!(Arc::ptr_eq(&result.rgba_data, &pixels));
    assert_eq!(request.validation_source, Some(source));
    assert!(
        request.cached_preview.is_none(),
        "validation queue must not retain pixel buffers"
    );
}

#[test]
fn folder_preview_cache_first_bypasses_known_invalidations_and_cancelled_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_at(&dir);
    let path = dir.path().join("unavailable-folder");
    put_snapshot(&cache, &path);
    let mut refresh = FolderPreviewRequest::new(path.clone(), 64, None).require_validation(true);
    assert!(cached_result(&mut refresh, FolderAppearance::Original, &cache).is_none());
    let mut requests = FolderPreviewRequests::default();
    let validity = requests.begin(&path);
    requests.invalidate(&path);
    let mut cancelled = FolderPreviewRequest::new(path, 64, None).with_validity(validity);
    assert!(cached_result(&mut cancelled, FolderAppearance::Original, &cache).is_none());
}

#[test]
fn folder_preview_cache_first_continues_when_validation_queue_is_full() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_at(&dir);
    let (requests_tx, requests_rx) = crossbeam_channel::bounded(4);
    let (slow_tx, slow_rx) = crossbeam_channel::bounded(1);
    let (results_tx, results_rx) = std::sync::mpsc::channel();
    // Simulate an indefinitely stalled Cryptomator validator. No source worker
    // consumes this queue, yet both cached images must still reach the UI.
    slow_tx
        .send(FolderPreviewRequest::new(
            PathBuf::from("busy-folder"),
            64,
            None,
        ))
        .unwrap();
    spawn_folder_preview_cache_worker(
        requests_rx,
        slow_tx,
        results_tx,
        egui::Context::default(),
        cache.clone(),
    );
    for index in 0..2 {
        let path = dir.path().join(format!("unavailable-folder-{index}"));
        put_snapshot(&cache, &path);
        requests_tx
            .send(FolderPreviewRequest::new(path, 64, None))
            .unwrap();
    }
    for _ in 0..2 {
        let image = results_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(image.validation_pending && !image.rgba_data.is_empty());
        let completion = results_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!completion.validation_pending && completion.retry_after_source_change);
    }
    assert_eq!(slow_rx.len(), 1);
    drop(requests_tx);
    assert!(matches!(
        results_rx.recv_timeout(Duration::from_secs(5)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn folder_preview_cache_first_miss_goes_to_source_queue() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_at(&dir);
    let (requests_tx, requests_rx) = crossbeam_channel::bounded(1);
    let (slow_tx, slow_rx) = crossbeam_channel::bounded(1);
    let (results_tx, results_rx) = std::sync::mpsc::channel();
    spawn_folder_preview_cache_worker(
        requests_rx,
        slow_tx,
        results_tx,
        egui::Context::default(),
        cache,
    );
    let path = dir.path().join("folder");
    requests_tx
        .send(FolderPreviewRequest::new(path.clone(), 64, None))
        .unwrap();
    let request = slow_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(request.path, path);
    assert_eq!(request.mode, FolderPreviewRequestMode::Load);
    assert!(results_rx.try_recv().is_err());
    drop(requests_tx);
    assert!(matches!(
        results_rx.recv_timeout(Duration::from_secs(5)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn folder_preview_cache_first_unchanged_source_needs_no_second_upload() {
    validate_snapshot(false);
}

#[test]
fn folder_preview_cache_first_changed_source_replaces_same_resolution() {
    validate_snapshot(true);
}

fn validate_snapshot(change_source: bool) {
    use crate::infrastructure::disk_cache::FolderPreviewPixels;
    use crate::infrastructure::folder_compose::FolderComposer;
    use crate::workers::folder_preview_worker::{
        spawn_folder_preview_worker, FolderPreviewTraceCounters,
    };
    let dir = tempfile::tempdir().unwrap();
    let cache = cache_at(&dir);
    let path = dir.path().join("folder");
    std::fs::create_dir(&path).unwrap();
    let cover = path.join("cover.jpg");
    std::fs::write(&cover, b"image").unwrap();
    let source = FolderPreviewSource::capture(&path, Some(&cover)).unwrap();
    let mut request = FolderPreviewRequest::new(path.clone(), 128, Some(cover.clone()));
    let old_pixels = [30, 60, 90, 255].repeat(128 * 128);
    cache.put_folder_preview_checked(
        &path,
        128,
        FolderPreviewPixels {
            rgba: &old_pixels,
            width: 128,
            height: 128,
        },
        FolderAppearance::Original,
        &source,
        &request.validity,
    );
    let first = cached_result(&mut request, FolderAppearance::Original, &cache).unwrap();
    if change_source {
        std::fs::write(&cover, b"changed image").unwrap();
        let updated = FolderPreviewSource::capture(&path, Some(&cover)).unwrap();
        assert_ne!(source, updated);
        // Another producer has already persisted fresh artwork. The validator
        // must replace the speculative image, not discard it for having equal size.
        let new_pixels = [200, 150, 100, 255].repeat(128 * 128);
        cache.put_folder_preview_checked(
            &path,
            128,
            FolderPreviewPixels {
                rgba: &new_pixels,
                width: 128,
                height: 128,
            },
            FolderAppearance::Original,
            &updated,
            &request.validity,
        );
    }
    let (source_tx, source_rx) = crossbeam_channel::bounded(1);
    let (results_tx, results_rx) = std::sync::mpsc::channel();
    let trace = Arc::new(FolderPreviewTraceCounters::default());
    spawn_folder_preview_worker(
        source_rx,
        results_tx,
        egui::Context::default(),
        cache,
        Arc::new(FolderComposer::try_new().unwrap()),
        trace.clone(),
    );
    source_tx.send(request).unwrap();
    let validated = results_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(!validated.validation_pending && validated.replace_existing);
    assert!(!validated.retry_after_source_change);
    if change_source {
        assert_eq!(
            (validated.width, validated.height),
            (first.width, first.height)
        );
        assert_ne!(validated.rgba_data, first.rgba_data);
        assert!(!validated.should_keep_texture([128, 128], false));
    } else {
        assert!(validated.rgba_data.is_empty());
    }
    assert_eq!(trace.take_snapshot().composes, 0);
    drop(source_tx);
    assert!(matches!(
        results_rx.recv_timeout(Duration::from_secs(5)),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
    ));
}
