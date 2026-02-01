use criterion::{criterion_group, criterion_main, Criterion};
use std::time::Instant;
// Note: This benchmark requires Windows to run as it depends on Windows Shell APIs.
// It serves as a verification tool for the performance impact of blocking operations.

#[cfg(target_os = "windows")]
use mtt_file_manager::infrastructure::windows::shell_operations;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

fn benchmark_shell_copy(c: &mut Criterion) {
    #[cfg(target_os = "windows")]
    c.bench_function("blocking_shell_copy", |b: &mut criterion::Bencher| {
        b.iter(|| {
            // Setup: Create a temporary file
            let temp_dir = std::env::temp_dir();
            let src = temp_dir.join("bench_test_src.txt");
            let dst = temp_dir.join("bench_test_dst.txt");
            std::fs::write(&src, "benchmark data").unwrap();

            // Measure the blocking call
            let start = Instant::now();
            let _ = shell_operations::copy_item_with_shell(&src, &temp_dir, HWND(std::ptr::null_mut()));
            let _duration = start.elapsed();

            // In a real scenario, we would assert duration < threshold if it was async,
            // but here we demonstrate it takes time (blocking).

            // Cleanup
            let _ = std::fs::remove_file(&src);
            let _ = std::fs::remove_file(&dst);
        })
    });
}

criterion_group!(benches, benchmark_shell_copy);
criterion_main!(benches);
