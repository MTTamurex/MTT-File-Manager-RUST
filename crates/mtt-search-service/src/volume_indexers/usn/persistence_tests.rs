use super::*;
use crate::file_index::VolumeIndex;
use crate::index_db::PersistedVolumeState;
use crate::volume_indexers::test_support::{index, TestDb};
use crate::volume_indices::handle_from;

#[test]
fn snapshot_captures_owned_rows_identity_and_checkpoint_before_draining() {
    let mut index = index();
    assert!(index.insert_record(20, "file 日本語.txt", 5, false, true));
    index.hardlink_parents.insert(20, vec![10]);
    index.pending_removals.insert(30);
    let handle = handle_from(index);
    let snapshot = take_pending_snapshot(&handle).unwrap();
    assert_eq!(snapshot.drive_letter, 'R');
    assert_eq!(snapshot.volume_id, "volume-id");
    assert_eq!(snapshot.volume_serial, 123);
    assert_eq!((snapshot.journal_id, snapshot.last_usn), (7, 42));
    assert_eq!(snapshot.files_indexed, 1);
    assert_eq!(
        snapshot.addition_rows,
        vec![(20, "file 日本語.txt".into(), 5, false, true, vec![10])]
    );
    assert!(snapshot.removals.contains(&30));
    assert!(handle.read().pending_additions.is_empty());
    assert!(handle.read().pending_removals.is_empty());
    handle.write().remove_record(20);
    assert_eq!(snapshot.addition_rows[0].1, "file 日本語.txt");
}

#[test]
fn invalid_pending_record_leaves_all_pending_sets_intact() {
    let mut index = index();
    assert!(index.insert_record(20, "file.txt", 5, false, false));
    index.pending_additions.insert(99);
    index.pending_removals.insert(30);
    let handle = handle_from(index);
    assert!(take_pending_snapshot(&handle).is_err());
    assert_eq!(handle.read().pending_additions.len(), 2);
    assert!(handle.read().pending_additions.contains(&99));
    assert!(handle.read().pending_removals.contains(&30));
}

#[test]
fn failed_transaction_rolls_back_rows_and_checkpoint_and_restores_pending_changes() {
    let fixture = TestDb::new();
    let mut index = index();
    assert!(index.insert_record(20, "old.txt", 5, false, false));
    assert!(index.insert_record(30, "removed.txt", 5, false, false));
    index.hardlink_parents.insert(30, vec![10]);
    fixture.db.save_volume(&index, |_, _| {}).unwrap();
    index.clear_pending();
    assert!(index.move_record(20, "new.txt", 10, false, true));
    index.hardlink_parents.insert(20, vec![11, 11, 0, 20, 10]);
    index.remove_record(30);
    index.last_usn = 100;
    let handle = handle_from(index);
    let snapshot = take_pending_snapshot(&handle).unwrap();
    // Fail at the final write, after row removals/additions and hardlink changes.
    fixture
        .sql
        .execute_batch(
            "CREATE TRIGGER fail_checkpoint BEFORE INSERT ON volume_state
        BEGIN SELECT RAISE(ABORT, 'injected checkpoint failure'); END;",
        )
        .unwrap();
    // New changes occurring after snapshot capture must survive restoration too.
    assert!(handle
        .write()
        .insert_record(40, "later.txt", 5, false, false));
    assert!(!persist_pending_snapshot_with_mode(
        &fixture.db,
        &handle,
        'R',
        snapshot,
        false
    ));
    assert_eq!(fixture.db.load_volume_state('R').unwrap().last_usn, 42);
    let mut loaded = VolumeIndex::empty('R');
    fixture.db.load_into_index(&mut loaded, |_| {}).unwrap();
    let old = loaded.records.get(&20).unwrap();
    assert_eq!(loaded.names.get(old.name_ref()), "old.txt");
    assert!(loaded.records.get(&30).is_some());
    assert_eq!(loaded.hardlink_parents.get(&30).unwrap(), &[10]);
    assert!(handle.read().pending_additions.contains(&20));
    assert!(handle.read().pending_additions.contains(&40));
    assert!(handle.read().pending_removals.contains(&30));

    fixture
        .sql
        .execute_batch("DROP TRIGGER fail_checkpoint")
        .unwrap();
    let retry = take_pending_snapshot(&handle).unwrap();
    assert!(persist_pending_snapshot_with_mode(
        &fixture.db,
        &handle,
        'R',
        retry,
        false
    ));
    let mut loaded = VolumeIndex::empty('R');
    fixture.db.load_into_index(&mut loaded, |_| {}).unwrap();
    assert_eq!(fixture.db.load_volume_state('R').unwrap().last_usn, 100);
    assert!(loaded.records.get(&30).is_none());
    assert!(!loaded.hardlink_parents.contains_key(&30));
    assert!(loaded.records.get(&40).is_some());
    assert!(loaded.reparse_points.contains(&20));
    assert_eq!(loaded.hardlink_parents.get(&20).unwrap(), &[11]);
    let updated = loaded.records.get(&20).unwrap();
    assert_eq!(loaded.names.get(updated.name_ref()), "new.txt");
    assert_eq!(updated.parent_ref, 10);
    assert!(handle.read().pending_additions.is_empty());
    assert!(handle.read().pending_removals.is_empty());
}

#[test]
fn checkpoint_only_persist_advances_state_but_reports_no_structural_changes() {
    let fixture = TestDb::new();
    let mut index = index();
    assert!(index.insert_record(20, "file.txt", 5, false, false));
    fixture.db.save_volume(&index, |_, _| {}).unwrap();
    index.clear_pending();
    index.last_usn = 99;
    let handle = handle_from(index);
    let snapshot = take_pending_snapshot(&handle).unwrap();
    assert!(!persist_pending_snapshot_with_mode(
        &fixture.db,
        &handle,
        'R',
        snapshot,
        false
    ));
    let state = fixture.db.load_volume_state('R').unwrap();
    assert_eq!((state.last_usn, state.files_indexed), (99, 1));
    assert_eq!(state.volume_id, "volume-id");
    assert_eq!(state.volume_serial, 123);
    assert!(state.has_hardlink_parent_data && state.has_reparse_point_data);
    let reopened =
        index_db::IndexDb::open(std::path::Path::new(fixture.sql.path().unwrap())).unwrap();
    assert_eq!(reopened.load_volume_state('R').unwrap().last_usn, 99);
    let mut loaded = VolumeIndex::empty('R');
    assert_eq!(
        reopened.load_into_index(&mut loaded, |_| {}).unwrap(),
        Some(1)
    );
}

#[test]
fn binary_only_mode_defers_checkpoint_until_metrics_are_durable() {
    for in_progress in [false, true] {
        let fixture = TestDb::new();
        let mut index = index();
        assert!(index.insert_record(20, "file.txt", 5, false, false));
        index.binary_dirty = true;
        index.sizes_loaded = true;
        if in_progress {
            index.size_refresh_in_progress = true;
        } else {
            index.pending_size_refresh.insert(20);
        }
        let handle = handle_from(index);
        let snapshot = take_pending_snapshot(&handle).unwrap();
        assert!(!persist_pending_snapshot_with_mode(
            &fixture.db,
            &handle,
            'R',
            snapshot,
            true
        ));
        assert!(fixture.db.load_volume_state('R').is_none());
        assert!(handle.read().pending_additions.contains(&20));
        assert!(handle.read().binary_dirty);
    }
}

#[test]
fn clean_binary_needs_no_io_and_unloaded_sizes_block_dirty_flush() {
    let mut index = index();
    index.binary_dirty = false;
    let handle = handle_from(index);
    assert!(flush_binary_snapshot_if_dirty(&handle, 'R'));
    handle.write().binary_dirty = true;
    handle.write().sizes_loaded = false;
    assert!(!flush_binary_snapshot_if_dirty(&handle, 'R'));
    assert!(handle.read().binary_dirty);
}

#[test]
fn sqlite_preference_respects_journal_freshness_and_binary_only_policy() {
    let state = || PersistedVolumeState {
        drive_letter: 'R',
        volume_id: "volume-id".into(),
        volume_serial: 123,
        journal_id: 7,
        last_usn: 42,
        files_indexed: 1,
        has_hardlink_parent_data: true,
        has_reparse_point_data: true,
    };
    for (sqlite_journal, sqlite_usn, binary_journal, binary_usn, skip, expected) in [
        (7, 50, 7, 42, false, true),
        (7, 50, 7, 42, true, false),
        (7, 42, 7, 50, false, false),
        (7, 42, 7, 42, false, false),
        (7, 10, 6, 100, false, true),
        (7, 10, 6, 100, true, true),
        (6, 100, 7, 10, false, false),
    ] {
        let sqlite = PersistedVolumeState {
            journal_id: sqlite_journal,
            last_usn: sqlite_usn,
            ..state()
        };
        let binary = PersistedVolumeState {
            journal_id: binary_journal,
            last_usn: binary_usn,
            ..state()
        };
        assert_eq!(prefer_sqlite_with_mode(&sqlite, &binary, 7, skip), expected);
    }
}
