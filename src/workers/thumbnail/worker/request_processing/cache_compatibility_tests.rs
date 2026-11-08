use super::*;

#[test]
fn existing_video_cache_hit_skips_source_io_and_both_extractors() {
    let directory = tempfile::tempdir().unwrap();
    let cache_dir = directory.path().join("cache");
    let source = directory.path().join("clip.mkv"); // Deliberately nonexistent.
    let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(42);
    {
        let cache = ThumbnailDiskCache::new(cache_dir.clone()).unwrap();
        cache.put(&source, modified, 512, &[255; 16], 2, 2).unwrap();
    }
    let cache = ThumbnailDiskCache::new(cache_dir).unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let (cache_write_tx, cache_write_rx) = crossbeam_channel::unbounded();
    process_thumbnail_request(
        &source,
        1,
        256,
        1,
        IOPriority::Interactive,
        42,
        &tx,
        &egui::Context::default(),
        &cache,
        &Semaphore::new(1),
        &dashmap::DashMap::new(),
        &mut Instant::now(),
        &cache_write_tx,
        &|| panic!("Cache hit must never enter extraction"),
    );
    let result = rx.try_recv().unwrap();
    assert!(!result.not_found);
    assert_eq!((result.width, result.height), (2, 2));
    assert!(!result.image_data.is_empty());
    assert!(
        cache_write_rx.is_empty(),
        "An existing entry must not be regenerated"
    );
    assert!(!source.exists());
}
