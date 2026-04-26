use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::path::{Path, PathBuf};

fn gather_images(root: &Path, out: &mut Vec<PathBuf>, limit: usize) {
    if out.len() >= limit {
        return;
    }

    let read = match std::fs::read_dir(root) {
        Ok(v) => v,
        Err(_) => return,
    };

    for entry in read.flatten() {
        if out.len() >= limit {
            break;
        }

        let path = entry.path();
        if path.is_dir() {
            gather_images(&path, out, limit);
            continue;
        }

        let is_image = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| {
                mtt_file_manager::infrastructure::windows::is_image_extension(ext)
                    && !ext.eq_ignore_ascii_case("svg")
            })
            .unwrap_or(false);

        if is_image {
            out.push(path);
        }
    }
}

fn decode_benchmark(c: &mut Criterion) {
    let root = std::env::var("MTT_BENCH_IMAGE_DIR").unwrap_or_else(|_| "assets".to_string());
    let root = PathBuf::from(root);

    if !root.exists() {
        return;
    }

    let mut images = Vec::new();
    gather_images(&root, &mut images, 12);

    if images.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("image_viewer_decode");

    for path in images {
        let label = path
            .file_name()
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_else(|| "image".to_string());

        group.bench_with_input(
            BenchmarkId::new("full", label.clone()),
            &path,
            |b, image_path| {
                b.iter(|| {
                    let _ = mtt_file_manager::image_viewer::decode_full_for_benchmark(image_path)
                        .expect("full decode should succeed");
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("preview", label),
            &path,
            |b, image_path| {
                b.iter(|| {
                    let _ = mtt_file_manager::image_viewer::decode_preview_for_benchmark(
                        image_path, 1440,
                    )
                    .expect("preview decode should succeed");
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, decode_benchmark);
criterion_main!(benches);
