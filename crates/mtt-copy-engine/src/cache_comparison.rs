//! Same engine, two cache modes. All measured copies finish before verification
//! reads; no source/destination paths, filenames or payload bytes are printed.
use crate::{cache::CacheMode, copy_with_cache, plan, Progress};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

fn checked<T>(result: io::Result<T>, stage: &str) -> T {
    result.unwrap_or_else(|error| {
        panic!(
            "{stage}: kind={:?} os_error={:?}",
            error.kind(),
            error.raw_os_error()
        )
    })
}

fn verify(source: &Path, target: &Path, size: u64) {
    let mut source = checked(File::open(source), "Open verification source");
    let mut target = checked(File::open(target), "Open verification target");
    let mut left = vec![0; 1024 * 1024];
    let mut right = vec![0; left.len()];
    let mut remaining = size;
    while remaining > 0 {
        let count = remaining.min(left.len() as u64) as usize;
        checked(
            source.read_exact(&mut left[..count]),
            "Read verification source",
        );
        checked(
            target.read_exact(&mut right[..count]),
            "Read verification target",
        );
        assert!(left[..count] == right[..count], "Copied contents differ");
        remaining -= count as u64;
    }
    assert_eq!(checked(source.read(&mut left[..1]), "Read source end"), 0);
    assert_eq!(checked(target.read(&mut right[..1]), "Read target end"), 0);
}

fn compare(source: &Path, destination: &Path, unbuffered_first: bool) {
    checked(plan::check_path(source), "Validate source ancestors");
    checked(
        plan::check_path(destination),
        "Validate destination ancestors",
    );
    let resolved = checked(crate::path_resolution::resolve(source), "Resolve source");
    let source = resolved.path;
    let before = checked(fs::metadata(&source), "Read source metadata");
    assert!(before.is_file(), "Comparison requires one regular file");
    let size = before.len();
    assert!(size > 0, "Comparison requires a nonempty file");
    let name = source.file_name().expect("Source must have a filename");
    let modes = if unbuffered_first {
        [CacheMode::Unbuffered, CacheMode::Buffered]
    } else {
        [CacheMode::Buffered, CacheMode::Unbuffered]
    };
    let mut outputs = Vec::new();
    let mut policy = None;
    for (index, mode) in modes.into_iter().enumerate() {
        let output = checked(
            tempfile::Builder::new()
                .prefix("mtt-copy-cache-")
                .tempdir_in(destination),
            "Create isolated comparison destination",
        );
        let progress = Progress::default();
        let start = Instant::now();
        let summary = checked(
            copy_with_cache(
                std::slice::from_ref(&source),
                output.path(),
                &progress,
                mode,
            ),
            "Run engine copy",
        );
        let elapsed = start.elapsed();
        let mode = match mode {
            CacheMode::Buffered => "buffered",
            CacheMode::Unbuffered => "unbuffered",
        };
        println!(
            "sample={} mode={mode} bytes={size} elapsed_ms={} peak_transfers={} failures={} cancelled={}",
            index + 1, elapsed.as_millis(),
            progress.telemetry.peak_active.load(Ordering::Relaxed),
            summary.failures, summary.cancelled,
        );
        assert_eq!(summary.failures, 0, "Comparison copy failed");
        assert!(!summary.cancelled);
        assert_eq!(summary.files_copied, 1);
        // CopyFileEx progress includes alternate streams; metadata.len() and
        // the byte-for-byte check below describe only the main data stream.
        assert!(progress.bytes_copied.load(Ordering::Relaxed) >= size);
        if let Some(policy) = policy {
            assert_eq!(progress.telemetry.policy(), Some(policy));
        } else {
            policy = progress.telemetry.policy();
            assert!(policy.is_some());
        }
        assert_eq!(
            checked(
                checked(fs::read_dir(output.path()), "List owned destination")
                    .try_fold(0usize, |count, entry| entry.map(|_| count + 1)),
                "Read owned destination entries",
            ),
            1
        );
        outputs.push(output);
    }
    // Verification deliberately runs outside both timers and after both copies,
    // rather than pre-reading the source immediately before the second sample.
    for output in outputs {
        verify(&source, &output.path().join(name), size);
        // TempDir's Drop ignores cleanup errors. Explicit close prevents a
        // successful comparison from silently leaving copied data behind.
        checked(output.close(), "Remove owned comparison destination");
    }
    let after = checked(fs::metadata(source), "Recheck source metadata");
    assert_eq!(after.len(), size, "Source size changed during comparison");
    assert!(
        before.modified().ok() == after.modified().ok(),
        "Source modification time changed"
    );
    println!("verification=passed source_metadata_unchanged=true");
}

#[test]
fn both_cache_modes_use_the_full_engine_and_preserve_small_unaligned_files() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let bytes: Vec<u8> = (0..2 * 1024 * 1024 + 37)
        .map(|n| ((n * 31 + n / 251) % 256) as u8)
        .collect();
    fs::write(&source, bytes).unwrap();
    for unbuffered_first in [false, true] {
        compare(&source, root.path(), unbuffered_first);
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn comparison_accepts_files_with_alternate_data_streams() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, b"contents").unwrap();
    fs::write(root.path().join("source.bin:metadata"), b"stream contents").unwrap();
    compare(&source, root.path(), false);
    assert_eq!(fs::read(source).unwrap(), b"contents");
    assert_eq!(
        fs::read(root.path().join("source.bin:metadata")).unwrap(),
        b"stream contents"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn comparison_cleans_owned_copies_of_readonly_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, b"contents").unwrap();
    let original = fs::metadata(&source).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&source, readonly).unwrap();
    let result = std::panic::catch_unwind(|| compare(&source, root.path(), false));
    let remained_readonly = fs::metadata(&source).unwrap().permissions().readonly();
    fs::set_permissions(&source, original).unwrap();
    assert!(result.is_ok(), "Comparison failed to clean readonly copies");
    assert!(remained_readonly);
    assert_eq!(fs::read(source).unwrap(), b"contents");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn comparison_modes_do_not_bypass_cancellation_or_existing_targets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let target = root.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("source.bin"), b"existing").unwrap();
    for mode in [CacheMode::Buffered, CacheMode::Unbuffered] {
        let progress = Progress::default();
        let error =
            copy_with_cache(std::slice::from_ref(&source), &target, &progress, mode).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        progress.cancel();
        let error =
            copy_with_cache(std::slice::from_ref(&source), &target, &progress, mode).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
    assert_eq!(fs::read(target.join("source.bin")).unwrap(), b"existing");
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
#[ignore = "Copies MTT_COPY_CACHE_SOURCE twice into owned temporary folders under MTT_COPY_CACHE_DESTINATION; run explicitly"]
fn compare_engine_cache_modes_on_selected_file() {
    let source = PathBuf::from(
        std::env::var_os("MTT_COPY_CACHE_SOURCE").expect("Set MTT_COPY_CACHE_SOURCE"),
    );
    let destination = PathBuf::from(
        std::env::var_os("MTT_COPY_CACHE_DESTINATION").expect("Set MTT_COPY_CACHE_DESTINATION"),
    );
    let unbuffered_first = match std::env::var("MTT_COPY_CACHE_ORDER").as_deref() {
        Ok("unbuffered-first") => true,
        Ok("buffered-first") | Err(std::env::VarError::NotPresent) => false,
        _ => panic!("MTT_COPY_CACHE_ORDER must be buffered-first or unbuffered-first"),
    };
    compare(&source, &destination, unbuffered_first);
}
