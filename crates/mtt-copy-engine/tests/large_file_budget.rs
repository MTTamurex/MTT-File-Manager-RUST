//! Modest integrity regression, not the user's heavy workload.
#[test]
fn same_volume_large_files_are_serialized_and_preserve_contents() {
    use std::sync::atomic::Ordering;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&target).unwrap();
    let data = vec![123u8; 1024 * 1024];
    for index in 0..5 {
        std::fs::write(source.join(format!("file-{index}.bin")), &data).unwrap();
    }
    let progress = mtt_copy_engine::Progress::default();
    let summary = mtt_copy_engine::copy(std::slice::from_ref(&source), &target, &progress).unwrap();
    assert_eq!(summary.files_copied, 5);
    assert_eq!(summary.failures, 0);
    assert_eq!(progress.telemetry.policy().unwrap().large_workers, 1);
    assert_eq!(progress.telemetry.peak_active.load(Ordering::Relaxed), 1);
    assert_eq!(progress.telemetry.active.load(Ordering::Relaxed), 0);
    assert_eq!(progress.telemetry.waiting.load(Ordering::Relaxed), 0);
    assert_eq!(progress.telemetry.phase(), mtt_copy_engine::Phase::Finished);
    for index in 0..5 {
        let name = format!("file-{index}.bin");
        assert_eq!(std::fs::read(source.join(&name)).unwrap(), data);
        assert_eq!(
            std::fs::read(target.join("source").join(name)).unwrap(),
            data
        );
    }
}
