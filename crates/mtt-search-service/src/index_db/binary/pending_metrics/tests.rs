use super::*;
use crate::file_index::FileMetrics;

fn sample() -> VolumeIndex {
    let mut index = VolumeIndex::empty('B');
    index.volume_id = r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".into();
    index.volume_serial = 123;
    index.journal_id = 7;
    index.last_usn = 42;
    index.hardlink_data_complete = true;
    index.reparse_data_complete = true;
    index.sizes_loaded = true;
    assert!(index.insert_record(5, "", 5, true, false));
    assert!(index.insert_record(10, "folder", 5, true, false));
    for frn in [20, 30] {
        assert!(index.insert_record(frn, "file.txt", 10, false, false));
        index.update_file_metrics(
            frn,
            FileMetrics {
                logical_size: 100,
                allocated_size: 4096,
            },
            std::time::Instant::now(),
        );
    }
    index.pending_size_refresh.insert(20);
    index
}

#[test]
fn pending_metrics_are_not_advertised_as_complete_to_older_readers() {
    let mut index = sample();
    let saved_flags = flags(&index);
    assert_eq!(saved_flags & COMPLETE_METRICS, 0);
    assert_ne!(saved_flags & SEQUENCE_AWARE_METRICS, 0);
    assert_ne!(saved_flags & PENDING_METRICS, 0);

    index.sizes_loaded = false;
    assert_eq!(flags(&index), 0);
    restore(&mut index, PENDING_METRICS);
    assert!(!index.sizes_loaded);
}

#[test]
fn repeated_restart_preserves_checkpoint_sizes_and_only_targeted_retries() {
    crate::index_db::init_data_dir_for_tests();
    let mut index = sample();
    for checkpoint in [50, 100, 150] {
        index.last_usn = checkpoint;
        super::super::save_and_remap(&mut index).unwrap();
        let (loaded, state) = super::super::load('B').unwrap().unwrap();
        assert_eq!(state.last_usn, checkpoint);
        assert!(
            loaded.sizes_loaded,
            "must not trigger a volume-wide size pass"
        );
        assert_eq!(loaded.records.len(), 4);
        assert_eq!(loaded.records.get(&30).unwrap().size, 100);
        assert_eq!(loaded.pending_size_refresh, [20].into_iter().collect());
        index = loaded;
    }

    // Once the targeted query succeeds, the next snapshot is fully complete.
    index.pending_size_refresh.remove(&20);
    super::super::save(&index).unwrap();
    let (loaded, _) = super::super::load('B').unwrap().unwrap();
    assert!(loaded.sizes_loaded);
    assert!(loaded.pending_size_refresh.is_empty());
    assert_ne!(flags(&loaded) & COMPLETE_METRICS, 0);
}

#[test]
fn drained_in_flight_metrics_cannot_be_saved_without_their_retry_ids() {
    crate::index_db::init_data_dir_for_tests();
    let mut index = sample();
    index.size_refresh_in_progress = true;
    assert!(super::super::save(&index).is_err());
}
