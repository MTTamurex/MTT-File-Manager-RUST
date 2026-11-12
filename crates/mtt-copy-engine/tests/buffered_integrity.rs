//! Opt-in production-path integrity check, not a performance benchmark.
#[cfg(windows)]
#[test]
#[ignore = "Writes a 256 MiB + 37 byte fixture and its copy; run on a disposable test volume"]
fn buffered_copy_preserves_large_contents_and_unaligned_tail() {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let destination = root.path().join("target");
    std::fs::create_dir(&destination).unwrap();
    let block: Vec<u8> = (0..1024 * 1024)
        .map(|index| ((index * 31 + index / 251) % 256) as u8)
        .collect();
    let tail = [93u8; 37];
    let mut fixture = std::fs::File::create(&source).unwrap();
    for _ in 0..256 {
        fixture.write_all(&block).unwrap();
    }
    fixture.write_all(&tail).unwrap();
    fixture.sync_all().unwrap();
    drop(fixture);

    let progress = mtt_copy_engine::Progress::default();
    let summary =
        mtt_copy_engine::copy(std::slice::from_ref(&source), &destination, &progress).unwrap();
    assert_eq!(summary.failures, 0, "{:?}", summary.first_error);
    assert!(!summary.cancelled);
    assert_eq!(summary.files_copied, 1);
    let size = 256 * 1024 * 1024 + tail.len() as u64;
    assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), size);
    assert_eq!(progress.telemetry.transferred.load(Ordering::Relaxed), size);
    for path in [source, destination.join("source.bin")] {
        let mut file = std::fs::File::open(path).unwrap();
        assert_eq!(file.metadata().unwrap().len(), size);
        let mut actual = vec![0; block.len()];
        for _ in 0..256 {
            file.read_exact(&mut actual).unwrap();
            assert_eq!(actual, block);
        }
        let mut actual_tail = [0; 37];
        file.read_exact(&mut actual_tail).unwrap();
        assert_eq!(actual_tail, tail);
        assert_eq!(file.read(&mut actual_tail).unwrap(), 0);
    }
    assert_eq!(std::fs::read_dir(destination).unwrap().count(), 1);
}
