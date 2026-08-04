use super::*;
use std::io::Write;
use std::num::NonZeroUsize;

fn cache() -> LiveFileSizeCache {
    LruCache::new(NonZeroUsize::new(8).unwrap())
}

#[test]
fn empty_cache_enqueues_a_request_for_a_nonzero_snapshot_without_tooltip_or_panel() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("new-file.bin");
    let mut cache = cache();
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();

    let size = resolve_cached_or_enqueue_live_file_size_at(
        &path,
        0,
        512,
        &mut cache,
        &mut active,
        &sender,
        now,
    );

    let request = receiver.try_recv().unwrap();
    assert_eq!(size, 512);
    assert_eq!(request.path, path);
    assert_eq!(request.source_mtime_secs, 0);
    assert_eq!(
        active.by_path.get(&path).unwrap().request_id,
        request.request_id
    );
}

#[test]
fn active_request_limit_bounds_column_probe_backlog() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut cache = cache();
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();

    for index in 0..(MAX_ACTIVE_LIVE_SIZE_REQUESTS + 1) {
        resolve_cached_or_enqueue_live_file_size_at(
            &temp_dir.path().join(format!("file-{index}.bin")),
            0,
            0,
            &mut cache,
            &mut active,
            &sender,
            now,
        );
    }

    assert_eq!(active.len(), MAX_ACTIVE_LIVE_SIZE_REQUESTS);
    assert_eq!(receiver.try_iter().count(), MAX_ACTIVE_LIVE_SIZE_REQUESTS);
}

#[test]
fn invalidation_does_not_release_the_outstanding_worker_limit() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut cache = cache();
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();
    let paths: Vec<_> = (0..MAX_ACTIVE_LIVE_SIZE_REQUESTS)
        .map(|index| temp_dir.path().join(format!("file-{index}.bin")))
        .collect();

    for path in &paths {
        resolve_cached_or_enqueue_live_file_size_at(
            path,
            0,
            0,
            &mut cache,
            &mut active,
            &sender,
            now,
        );
        invalidate_live_file_size(path, &mut cache, &mut active);
    }
    resolve_cached_or_enqueue_live_file_size_at(
        &temp_dir.path().join("blocked.bin"),
        0,
        0,
        &mut cache,
        &mut active,
        &sender,
        now,
    );

    assert_eq!(active.len(), MAX_ACTIVE_LIVE_SIZE_REQUESTS);
    let requests: Vec<_> = receiver.try_iter().collect();
    assert_eq!(requests.len(), MAX_ACTIVE_LIVE_SIZE_REQUESTS);

    assert_eq!(
        accept_live_file_size_response(
            LiveFileSizeResponse {
                path: requests[0].path.clone(),
                source_mtime_secs: requests[0].source_mtime_secs,
                request_id: requests[0].request_id,
                observed: None,
            },
            &mut cache,
            &mut active,
            now,
        ),
        None
    );
    assert_eq!(active.len(), MAX_ACTIVE_LIVE_SIZE_REQUESTS - 1);

    resolve_cached_or_enqueue_live_file_size_at(
        &temp_dir.path().join("unblocked.bin"),
        0,
        0,
        &mut cache,
        &mut active,
        &sender,
        now,
    );
    assert!(receiver.try_recv().is_ok());
}

#[test]
fn repeated_observations_capture_growth_with_the_same_source_mtime_and_then_settle() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("growing-file.bin");
    let mut cache = cache();
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let start = Instant::now();
    let observed_mtime = Some(SystemTime::now());
    let sizes = [0, 1024, 4096, 4096, 4096];

    for (index, size) in sizes.into_iter().enumerate() {
        let now = start + LIVE_SIZE_REVALIDATE_INTERVAL * index as u32;
        let displayed = resolve_cached_or_enqueue_live_file_size_at(
            &path,
            0,
            0,
            &mut cache,
            &mut active,
            &sender,
            now,
        );
        if index > 0 {
            assert_eq!(displayed, sizes[index - 1]);
        }

        let request = receiver.try_recv().unwrap();
        assert_eq!(
            accept_live_file_size_response(
                LiveFileSizeResponse {
                    path: path.clone(),
                    source_mtime_secs: 0,
                    request_id: request.request_id,
                    observed: Some(ObservedLiveFileSize {
                        size,
                        modified: observed_mtime,
                    }),
                },
                &mut cache,
                &mut active,
                now,
            ),
            Some(index + 1 < sizes.len())
        );
    }

    assert_eq!(cached_live_file_size(&path, 0, &cache), Some(4096));
    assert_eq!(
        resolve_cached_or_enqueue_live_file_size_at(
            &path,
            0,
            0,
            &mut cache,
            &mut active,
            &sender,
            start + LIVE_SIZE_REVALIDATE_INTERVAL * sizes.len() as u32,
        ),
        4096
    );
    assert!(receiver.try_recv().is_err());
}

#[test]
fn invalidated_response_cannot_replace_a_newer_request() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("recreated-file.bin");
    let mut cache = cache();
    let mut active = ActiveLiveFileSizeRequests::default();
    let (sender, receiver) = mpsc::channel();
    let now = Instant::now();

    resolve_cached_or_enqueue_live_file_size_at(&path, 0, 0, &mut cache, &mut active, &sender, now);
    let stale_request = receiver.try_recv().unwrap();
    invalidate_live_file_size(&path, &mut cache, &mut active);
    resolve_cached_or_enqueue_live_file_size_at(&path, 0, 0, &mut cache, &mut active, &sender, now);
    let current_request = receiver.try_recv().unwrap();

    assert_eq!(
        accept_live_file_size_response(
            LiveFileSizeResponse {
                path: path.clone(),
                source_mtime_secs: 0,
                request_id: stale_request.request_id,
                observed: Some(ObservedLiveFileSize {
                    size: 1,
                    modified: Some(SystemTime::now()),
                }),
            },
            &mut cache,
            &mut active,
            now,
        ),
        None
    );
    assert_eq!(
        active.by_path.get(&path).unwrap().request_id,
        current_request.request_id
    );

    assert!(accept_live_file_size_response(
        LiveFileSizeResponse {
            path: path.clone(),
            source_mtime_secs: 0,
            request_id: current_request.request_id,
            observed: Some(ObservedLiveFileSize {
                size: 8192,
                modified: Some(SystemTime::now()),
            }),
        },
        &mut cache,
        &mut active,
        now,
    )
    .is_some());
    assert_eq!(cached_live_file_size(&path, 0, &cache), Some(8192));
}

#[test]
fn worker_stat_reads_size_and_precise_modified_time_from_the_file() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&[7; 4096]).unwrap();
    file.flush().unwrap();
    let metadata = file.as_file().metadata().unwrap();

    let observed = read_live_file_size(file.path()).unwrap();

    assert_eq!(observed.size, 4096);
    assert_eq!(observed.modified, metadata.modified().ok());
}
