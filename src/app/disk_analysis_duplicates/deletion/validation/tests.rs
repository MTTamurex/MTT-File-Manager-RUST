use super::*;

#[test]
fn full_hash_reports_actual_bytes_and_preserves_the_digest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    let contents = vec![7; READ_CHUNK + 17];
    std::fs::write(&path, &contents).unwrap();
    let (mut file, stamp) = reader::open(&path).unwrap();
    let shared = Mutex::new(DeleteProgress::default());
    let mut progress = Reporter::new(&shared);
    progress.begin(DeleteStage::TargetContent, stamp.size);
    let digest = hash(&mut file, stamp, &AtomicBool::new(false), &mut progress).unwrap();
    assert_eq!(digest, *blake3::hash(&contents).as_bytes());
    let snapshot = super::super::progress::snapshot(&shared);
    assert_eq!(snapshot.bytes_read, contents.len() as u64);
    assert_eq!(snapshot.total_bytes, stamp.size);
    assert_eq!(snapshot.fraction(), Some(1.0));
}

#[test]
fn cancelled_hash_does_not_read_or_report_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.bin");
    std::fs::write(&path, [7; 10]).unwrap();
    let (mut file, stamp) = reader::open(&path).unwrap();
    let shared = Mutex::new(DeleteProgress::default());
    let mut progress = Reporter::new(&shared);
    progress.begin(DeleteStage::SurvivorContent, stamp.size);
    assert!(hash(&mut file, stamp, &AtomicBool::new(true), &mut progress).is_err());
    assert_eq!(super::super::progress::snapshot(&shared).bytes_read, 0);
    use std::io::Seek;
    assert_eq!(file.stream_position().unwrap(), 0);
}
