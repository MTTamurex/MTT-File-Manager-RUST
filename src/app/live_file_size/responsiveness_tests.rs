use super::*;
use std::num::NonZeroUsize;

#[test]
fn ui_enqueues_without_classifying_the_volume() {
    // A synthetic UNC path must be queued without querying its provider. The
    // worker is responsible for rejecting network/virtual paths before stat.
    let path = PathBuf::from(r"\\server\share\file.bin");
    let mut cache = LiveFileSizeCache::new(NonZeroUsize::new(8).unwrap());
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    assert_eq!(
        resolve_cached_or_enqueue_live_file_size_at(
            &path,
            0,
            512,
            &mut cache,
            &mut active,
            &sender,
            Instant::now(),
        ),
        512,
    );
    assert!(
        receiver.try_recv().is_ok(),
        "Volume classification ran before enqueue"
    );
}

#[test]
fn stalled_volume_check_does_not_block_rendering_or_duplicate_requests() {
    let path = PathBuf::from(r"\\server\share\file.bin");
    let mut cache = LiveFileSizeCache::new(NonZeroUsize::new(8).unwrap());
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();
    resolve_cached_or_enqueue_live_file_size_at(
        &path,
        0,
        512,
        &mut cache,
        &mut active,
        &sender,
        now,
    );
    let request = receiver.try_recv().unwrap();
    let (started, started_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            observe_live_file_size_with(
                &request,
                |_, _| {
                    started.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    false
                },
                |_| panic!("Rejected volume must not reach file metadata"),
            )
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let ctx = eframe::egui::Context::default();
        let mut frames = 0;
        for _ in 0..3 {
            let _ = ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
                // Match the preview/tooltip path, which runs inside data_mut.
                let size = ui.ctx().data_mut(|_| {
                    resolve_cached_or_enqueue_live_file_size_at(
                        &path,
                        0,
                        512,
                        &mut cache,
                        &mut active,
                        &sender,
                        now,
                    )
                });
                ui.label(size.to_string());
                frames += 1;
            });
        }
        let no_duplicates = receiver.try_recv().is_err();
        let outstanding = active.len();
        release.send(()).unwrap();
        assert!(worker.join().unwrap().is_none());
        assert!(frames >= 3);
        assert!(no_duplicates);
        assert_eq!(outstanding, 1);
    });
}

#[test]
fn worker_still_rejects_network_paths_before_reading_metadata() {
    let request = LiveFileSizeRequest {
        path: PathBuf::from(r"\\server\share\file.bin"),
        source_mtime_secs: 0,
        probe_mtime_secs: 0,
        request_id: 1,
    };
    assert!(
        observe_live_file_size_with(&request, should_probe_live_file_size, |_| panic!(
            "Network path reached file metadata"
        ),)
        .is_none()
    );
}

#[test]
fn old_listing_timestamp_is_not_reused_as_the_workers_freshness_timestamp() {
    let path = PathBuf::from(r"\\server\share\file.bin");
    let mut cache = LiveFileSizeCache::new(NonZeroUsize::new(8).unwrap());
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();
    let modified = SystemTime::now();
    let epoch = modified.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let listing_epoch = epoch - LIVE_SIZE_PROBE_MAX_AGE_SECS - 60;
    cache.put(
        path.clone(),
        LiveFileSizeCacheEntry {
            source_mtime_secs: listing_epoch,
            observed: Some(ObservedLiveFileSize {
                size: 1024,
                modified: Some(modified),
            }),
            checked_at: now - LIVE_SIZE_REVALIDATE_INTERVAL,
            stable_observations: 0,
        },
    );
    assert_eq!(
        resolve_cached_or_enqueue_live_file_size_at(
            &path,
            listing_epoch,
            512,
            &mut cache,
            &mut active,
            &sender,
            now,
        ),
        1024
    );
    let request = receiver.try_recv().unwrap();
    assert_eq!(request.source_mtime_secs, listing_epoch);
    assert_eq!(request.probe_mtime_secs, epoch);
    let observed = observe_live_file_size_with(
        &request,
        |_, timestamp| {
            assert_eq!(timestamp, epoch);
            is_recent_for_live_file_size(timestamp)
        },
        |_| {
            Some(ObservedLiveFileSize {
                size: 2048,
                modified: Some(modified),
            })
        },
    );
    assert!(observed.is_some());
    assert!(accept_live_file_size_response(
        LiveFileSizeResponse {
            path: path.clone(),
            source_mtime_secs: request.source_mtime_secs,
            request_id: request.request_id,
            observed,
        },
        &mut cache,
        &mut active,
        now
    )
    .is_some());
    assert_eq!(
        cached_live_file_size(&path, listing_epoch, &cache),
        Some(2048)
    );
}

#[test]
fn old_files_are_not_enqueued_and_rejected_results_are_cached() {
    let path = PathBuf::from(r"\\server\share\file.bin");
    let mut cache = LiveFileSizeCache::new(NonZeroUsize::new(8).unwrap());
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let old = epoch - LIVE_SIZE_PROBE_MAX_AGE_SECS - 60;
    resolve_cached_or_enqueue_live_file_size_at(
        &path,
        old,
        512,
        &mut cache,
        &mut active,
        &sender,
        now,
    );
    assert!(receiver.try_recv().is_err());
    assert!(active.is_empty());
    resolve_cached_or_enqueue_live_file_size_at(
        &path,
        0,
        512,
        &mut cache,
        &mut active,
        &sender,
        now,
    );
    let request = receiver.try_recv().unwrap();
    accept_live_file_size_response(
        LiveFileSizeResponse {
            path: path.clone(),
            source_mtime_secs: 0,
            request_id: request.request_id,
            observed: None,
        },
        &mut cache,
        &mut active,
        now,
    )
    .unwrap();
    assert!(active.is_empty());
    assert_eq!(
        resolve_cached_or_enqueue_live_file_size_at(
            &path,
            0,
            512,
            &mut cache,
            &mut active,
            &sender,
            now + LIVE_SIZE_REVALIDATE_INTERVAL - Duration::from_nanos(1),
        ),
        512
    );
    assert!(receiver.try_recv().is_err());
}
