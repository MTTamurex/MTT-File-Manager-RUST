//! Opt-in synthetic timings: no user media or application database is opened.
use super::*;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const ITEMS: usize = 32;
const PASSES: usize = 1;

#[test]
fn folder_preview_concurrent_cache_hits_preserve_pixels_and_source_validation() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailDiskCache::new(dir.path().join("cache")).unwrap();
    let mut entries = Vec::new();
    for index in 0..4u8 {
        let folder = dir.path().join(format!("folder-{index}"));
        std::fs::create_dir(&folder).unwrap();
        let source = FolderPreviewSource::capture(&folder, None).unwrap();
        let pixels = [index * 40, 80, 120, 128].repeat(64 * 64);
        cache.put_folder_preview_cache(&folder, 64, &pixels, 64, 64);
        let expected = [32, 64].map(|bucket| {
            cache
                .get_folder_preview_for_source(&folder, bucket, Default::default(), &source)
                .unwrap()
        });
        entries.push((folder, source, expected));
    }
    std::thread::scope(|scope| {
        for (folder, source, expected) in &entries {
            let cache = &cache;
            scope.spawn(move || {
                for _ in 0..8 {
                    for (bucket, expected) in [32, 64].into_iter().zip(expected) {
                        let actual = cache
                            .get_folder_preview_for_source(
                                folder,
                                bucket,
                                Default::default(),
                                source,
                            )
                            .unwrap();
                        assert_eq!(&actual, expected);
                    }
                }
            });
        }
    });
    // A stale source must still be rejected after the lock-scope optimization.
    for (folder, source, _) in &entries {
        std::fs::write(folder.join("file.txt"), b"changed").unwrap();
        assert!(cache
            .get_folder_preview_for_source(folder, 32, Default::default(), source)
            .is_none());
    }
}

fn elapsed_batch(
    cache: &ThumbnailDiskCache,
    paths: &[PathBuf],
    workers: usize,
    folders: bool,
    bucket: u32,
) -> Duration {
    let ready = Arc::new(Barrier::new(workers + 1));
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let ready = ready.clone();
            scope.spawn(move || {
                ready.wait();
                for _ in 0..PASSES {
                    for index in (worker..paths.len()).step_by(workers) {
                        let pixels = if folders {
                            cache
                                .get_folder_preview_cache(&paths[index], bucket)
                                .unwrap()
                                .0
                        } else {
                            let entry = cache.get_latest(&paths[index]).unwrap();
                            let decoded = webp::Decoder::new(&entry.data).decode().unwrap();
                            let rgba = decoded.to_image().to_rgba8();
                            super::resize_rgba_to_width(rgba.into_raw(), 512, 512, bucket)
                                .unwrap()
                                .0
                        };
                        std::hint::black_box(pixels);
                    }
                }
            });
        }
        ready.wait();
        let start = Instant::now();
        // Joining is included, but worker startup is excluded by the barrier.
        start
    })
    .elapsed()
}

#[test]
#[ignore = "opt-in synthetic warm-cache timing; run with --ignored --nocapture"]
fn folder_preview_warm_cache_timing() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ThumbnailDiskCache::new(dir.path().to_path_buf()).unwrap();
    let paths: Vec<_> = (0..ITEMS)
        .map(|i| PathBuf::from(format!("folder-{i}")))
        .collect();
    let pixels: Vec<_> = (0..512u32)
        .flat_map(|y| {
            (0..512u32).flat_map(move |x| {
                [
                    x as u8,
                    y as u8,
                    (x ^ y) as u8,
                    if x < 32 { 96 } else { 255 },
                ]
            })
        })
        .collect();
    let encoded = webp::Encoder::from_rgba(&pixels, 512, 512).encode(FOLDER_PREVIEW_WEBP_QUALITY);
    {
        let db = cache.writer.lock();
        for path in &paths {
            db.execute(
                "INSERT INTO folder_preview_variants
                 (folder_path, data, width, height, bucket_size, format_version, created_at)
                 VALUES (?1, ?2, 512, 512, 512, ?3, 0)",
                params![
                    path.to_string_lossy(),
                    encoded.to_vec(),
                    FOLDER_PREVIEW_CACHE_FORMAT_VERSION
                ],
            )
            .unwrap();
            db.execute(
                "INSERT INTO thumbnails
                 (id, path, data, width, height, requested_size, modified_at, created_at)
                 VALUES (?1, ?2, ?3, 512, 512, 512, 0, 0)",
                params![
                    ThumbnailDiskCache::hash_path(path),
                    path.to_string_lossy(),
                    encoded.to_vec()
                ],
            )
            .unwrap();
        }
    }
    for bucket in [512, 256] {
        for workers in [1, 4] {
            for folders in [false, true] {
                // Warm the SQLite/OS pages before measuring.
                let _ = elapsed_batch(&cache, &paths, workers, folders, bucket);
                let elapsed = elapsed_batch(&cache, &paths, workers, folders, bucket);
                eprintln!(
                    "warm_cache kind={} workers={} bucket={} items={} ms={:.2}",
                    if folders { "folders" } else { "files" },
                    workers,
                    bucket,
                    ITEMS * PASSES,
                    elapsed.as_secs_f64() * 1000.0,
                );
            }
        }
    }
}
