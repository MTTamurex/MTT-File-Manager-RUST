use super::*;
use crate::file_index::{FileMetrics, VolumeIndex};

fn cached() -> VolumeIndex {
    let mut index = VolumeIndex::empty('R');
    assert!(index.insert_record(5, "", 5, true, false));
    assert!(index.insert_record(10, "folder", 5, true, false));
    assert!(index.insert_record(20, "file.bin", 5, false, false));
    assert!(index.insert_record(21, "file.bin", 10, false, false));
    index.records.get_mut(&21).unwrap().size = 500;
    index.records.get_mut(&21).unwrap().allocated_size = 8192;
    index.binary_dirty = false;
    index
}

#[test]
fn legacy_size_repair_restores_zero_and_shrunk_values_for_every_consumer() {
    let mut index = cached();
    let metrics = FileMetrics {
        logical_size: 100,
        allocated_size: 4096,
    };
    assert_eq!(
        apply_sizes(&mut index, &[(20, metrics), (21, metrics)], || false),
        Some(2)
    );
    assert!(index.sizes_loaded);
    assert!(index.binary_dirty);
    let root = index.folder_tree_summary(5);
    let folder = index.folder_tree_summary(10);
    assert_eq!((root.0, root.1, root.3), (200, 2, 0));
    assert_eq!((folder.0, folder.1, folder.3), (100, 1, 0));
    assert!(index.dir_modified_at.contains_key(&5));
    assert!(index.dir_modified_at.contains_key(&10));
    index.state = crate::file_index::IndexState::Ready;
    let handle = crate::volume_indices::handle_from(index);
    let snapshot = crate::disk_analysis::build_snapshot(&handle).unwrap();
    let files: Vec<_> = snapshot.records.iter().filter(|r| !r.is_dir).collect();
    assert_eq!(files.len(), 2);
    assert!(files
        .iter()
        .all(|r| r.size == 100 && r.allocated_size == 4096));
    let results = crate::file_index::search_page(&[handle], "file.bin", 0, 10);
    assert_eq!(results.items.len(), 2);
    assert!(results.items.iter().all(|r| r.size == 100));
}

#[test]
fn journal_changes_pending_during_the_bulk_pass_win_over_older_samples() {
    let mut index = cached();
    index.pending_size_refresh.insert(21);
    assert_eq!(
        apply_sizes(&mut index, &[(21, FileMetrics::default())], || false),
        Some(0)
    );
    assert_eq!(index.records.get(&21).unwrap().size, 500);
    assert!(index.pending_size_refresh.contains(&21));
    assert!(
        index.sizes_loaded,
        "live pending queries can run after the bulk pass"
    );
}

#[test]
fn genuine_zero_is_applied_but_cancellation_does_not_mark_sizes_complete() {
    let mut index = cached();
    assert_eq!(
        apply_sizes(&mut index, &[(21, FileMetrics::default())], || true),
        None
    );
    assert!(!index.sizes_loaded);
    assert_eq!(index.records.get(&21).unwrap().size, 500);
    assert_eq!(
        apply_sizes(&mut index, &[(21, FileMetrics::default())], || false),
        Some(1)
    );
    assert_eq!(index.records.get(&21).unwrap().size, 0);
    assert_eq!(index.records.get(&21).unwrap().allocated_size, 0);
}

#[test]
fn cancellation_after_partial_application_keeps_sizes_unloaded_and_changes_dirty() {
    let mut index = cached();
    let checks = std::cell::Cell::new(0);
    assert_eq!(
        apply_sizes(&mut index, &[(21, FileMetrics::default())], || {
            let check = checks.get();
            checks.set(check + 1);
            check > 0
        }),
        None
    );
    assert!(!index.sizes_loaded);
    assert!(index.binary_dirty);
    assert_eq!(index.records.get(&21).unwrap().size, 0);
}
