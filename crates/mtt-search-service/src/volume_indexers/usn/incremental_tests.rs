use super::{apply_incremental_records, next_journal_poll_delay};
use crate::volume_indexers::test_support::{index, usn_record};

fn named(frn: u64, parent: u64, reason: u32, name: &str) -> Vec<u8> {
    usn_record(
        frn,
        parent,
        reason,
        &name.encode_utf16().collect::<Vec<_>>(),
    )
}

#[test]
fn journal_backlog_is_drained_without_sleeping_between_buffers() {
    use std::time::Duration;

    assert_eq!(next_journal_poll_delay(42, 50), Duration::ZERO);
    assert_eq!(next_journal_poll_delay(50, 60), Duration::ZERO);
    assert_eq!(next_journal_poll_delay(60, 60), Duration::from_secs(2));
    // Failed/contended reads must retain the idle delay rather than busy-loop.
    assert_eq!(next_journal_poll_delay(42, 42), Duration::from_secs(2));
}

#[test]
fn multi_buffer_deletion_updates_every_ancestor_after_the_last_buffer() {
    let mut index = index();
    assert!(index.insert_record(10, "Folder", 5, true, false));
    assert!(index.insert_record(11, "Nested", 10, true, false));
    for frn in [20, 21] {
        assert!(index.insert_record(frn, "file.bin", 11, false, false));
        index.records.get_mut(&frn).unwrap().size = 10;
    }
    let mut cursor = index.last_usn;
    apply_incremental_records(
        &mut index,
        &named(20, 11, 0x200, "file.bin"),
        50,
        &mut cursor,
    )
    .unwrap();
    for ancestor in [5, 10, 11] {
        assert_eq!(index.folder_tree_summary(ancestor).0, 10);
    }
    // This already-changed total is still partial; the next buffer must not wait.
    assert!(index.journal_refresh_in_progress);
    assert!(next_journal_poll_delay(42, cursor).is_zero());
    apply_incremental_records(
        &mut index,
        &named(21, 11, 0x200, "file.bin"),
        60,
        &mut cursor,
    )
    .unwrap();
    for ancestor in [5, 10, 11] {
        assert_eq!(index.folder_tree_summary(ancestor).0, 0);
    }
    assert!(index.journal_refresh_in_progress);
    // A successful empty read with no cursor advance establishes the tail.
    apply_incremental_records(&mut index, &[], 60, &mut cursor).unwrap();
    assert!(!index.journal_refresh_in_progress);
}

#[test]
fn create_rename_and_delete_advance_both_cursors_and_track_deltas() {
    let mut index = index();
    let mut cursor = index.last_usn;
    apply_incremental_records(
        &mut index,
        &named(20, 5, 0x100, "file.txt"),
        50,
        &mut cursor,
    )
    .unwrap();
    assert_eq!((index.last_usn, cursor), (50, 50));
    assert!(index.pending_additions.contains(&20));
    assert!(index.pending_size_refresh.contains(&20));
    index.clear_pending();
    apply_incremental_records(
        &mut index,
        &named(20, 10, 0x2000, "renamed.txt"),
        60,
        &mut cursor,
    )
    .unwrap();
    let record = index.records.get(&20).unwrap();
    assert_eq!(record.parent_ref, 10);
    assert_eq!(index.names.get(record.name_ref()), "renamed.txt");
    assert!(index.dir_modified_at.contains_key(&5));
    assert!(index.dir_modified_at.contains_key(&10));
    assert!(index.pending_additions.contains(&20));
    index.clear_pending();
    apply_incremental_records(
        &mut index,
        &named(20, 10, 0x200, "renamed.txt"),
        70,
        &mut cursor,
    )
    .unwrap();
    assert!(index.records.get(&20).is_none());
    assert!(index.pending_removals.contains(&20));
    assert_eq!((index.last_usn, cursor), (70, 70));
}

#[test]
fn failed_batch_keeps_checkpoint_and_can_be_replayed_after_partial_apply() {
    let mut index = index();
    let mut cursor = index.last_usn;
    let valid = named(20, 5, 0x100, "file.txt");
    let mut batch = valid.clone();
    batch.extend(usn_record(
        21,
        5,
        0x100,
        &vec![0xd800; u16::MAX as usize / 2],
    ));
    assert!(apply_incremental_records(&mut index, &batch, 100, &mut cursor).is_err());
    assert!(index.journal_refresh_in_progress);
    assert_eq!((index.last_usn, cursor), (42, 42));
    assert!(index.records.get(&20).is_some());
    assert!(index.records.get(&21).is_none());
    let arena_len = index.names.len();
    // Retry the already-applied prefix plus a representable second record.
    let mut retry = valid;
    retry.extend(named(21, 5, 0x100, "other.txt"));
    apply_incremental_records(&mut index, &retry, 100, &mut cursor).unwrap();
    assert_eq!(index.records.len(), 2);
    assert_eq!(index.names.len(), arena_len + "other.txt".len());
    assert_eq!((index.last_usn, cursor), (100, 100));
    assert_eq!(index.pending_additions.len(), 2);
}

#[test]
fn empty_batch_advances_checkpoint_without_structural_changes() {
    let mut index = index();
    let mut cursor = 42;
    apply_incremental_records(&mut index, &[], 50, &mut cursor).unwrap();
    assert_eq!((index.last_usn, cursor), (50, 50));
    assert!(index.pending_additions.is_empty());
    assert!(index.pending_removals.is_empty());
}

#[test]
fn combined_rename_and_size_change_queues_metrics_without_duplicate_records() {
    let mut index = index();
    assert!(index.insert_record_untracked(20, "file.txt", 5, false, false));
    index.clear_pending();
    let mut cursor = 42;
    let record = named(20, 10, 0x2000 | 0x2, "renamed.txt");
    apply_incremental_records(&mut index, &record, 50, &mut cursor).unwrap();
    apply_incremental_records(&mut index, &record, 60, &mut cursor).unwrap();
    assert_eq!(index.records.len(), 1);
    assert_eq!(index.pending_size_refresh.len(), 1);
    assert!(index.pending_size_refresh.contains(&20));
    assert_eq!((index.last_usn, cursor), (60, 60));
}
