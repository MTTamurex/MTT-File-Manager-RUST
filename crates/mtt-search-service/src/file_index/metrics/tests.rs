use super::*;

#[test]
fn changed_orphan_metrics_invalidate_the_normalized_volume_root() {
    let mut index = VolumeIndex::empty('R');
    assert!(index.insert_record(5, "", 5, true, false));
    assert!(index.insert_record(20, "file.bin", 0, false, false));
    let metrics = FileMetrics {
        logical_size: 100,
        allocated_size: 4096,
    };
    let now = Instant::now();
    assert!(index.update_file_metrics(20, metrics, now));
    assert_eq!(index.dir_modified_at.get(&5), Some(&now));
    index.binary_dirty = false;
    assert!(!index.update_file_metrics(20, metrics, now));
    assert!(!index.binary_dirty);
    assert!(!index.update_file_metrics(5, metrics, now));
    assert!(!index.update_file_metrics(99, metrics, now));
}

#[test]
fn ancestor_invalidation_terminates_on_malformed_directory_cycles() {
    let mut index = VolumeIndex::empty('R');
    assert!(index.insert_record(10, "folder", 11, true, false));
    assert!(index.insert_record(11, "nested", 10, true, false));
    assert!(index.insert_record(20, "file.bin", 10, false, false));
    assert!(index.update_file_metrics(
        20,
        FileMetrics {
            logical_size: 1,
            allocated_size: 0
        },
        Instant::now()
    ));
    assert_eq!(index.dir_modified_at.len(), 2);
}
