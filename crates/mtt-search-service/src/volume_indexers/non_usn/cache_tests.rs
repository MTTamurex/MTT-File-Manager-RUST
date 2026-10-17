use super::*;
use crate::volume_indexers::test_support::{index, TestDb};

fn volume() -> IndexVolume {
    IndexVolume {
        drive_letter: 'R',
        volume_id: "VOLUME-ID".into(),
        volume_serial: 123,
        file_system: "exFAT".into(),
    }
}

#[test]
fn matching_cache_loads_unicode_records_reparse_and_hardlinks_as_ready() {
    let fixture = TestDb::new();
    let mut source = index();
    assert!(source.insert_record(20, "file 日本語.txt", 5, false, true));
    source.hardlink_parents.insert(20, vec![10]);
    fixture.db.save_volume(&source, |_, _| {}).unwrap();
    let volume = volume();
    let estimate = matching_record_estimate(&fixture.db, &volume);
    assert_eq!(estimate, Some(1));
    let cached = load_cached_index(&fixture.db, &volume, estimate).unwrap();
    assert!(matches!(cached.state, IndexState::Ready));
    assert_eq!((cached.journal_id, cached.last_usn), (0, 0));
    assert_eq!(cached.volume_serial, 123);
    assert_eq!(cached.volume_id, volume.volume_id);
    let record = cached.records.get(&20).unwrap();
    assert_eq!(cached.names.get(record.name_ref()), "file 日本語.txt");
    assert!(cached.reparse_points.contains(&20));
    assert_eq!(cached.hardlink_parents.get(&20).unwrap(), &[10]);
}

#[test]
fn stale_rows_are_not_loaded_without_matching_metadata() {
    let fixture = TestDb::new();
    let mut source = index();
    assert!(source.insert_record(20, "file.txt", 5, false, false));
    fixture.db.save_volume(&source, |_, _| {}).unwrap();
    for (id, serial) in [
        ("different-volume", 123),
        ("volume-id", 456),
        ("volume-id", 0),
    ] {
        let mut volume = volume();
        volume.volume_id = id.into();
        volume.volume_serial = serial;
        let estimate = matching_record_estimate(&fixture.db, &volume);
        assert_eq!(estimate, None);
        assert!(load_cached_index(&fixture.db, &volume, estimate).is_none());
    }
    fixture.sql.execute("DELETE FROM volume_state", []).unwrap();
    let volume = volume();
    let estimate = matching_record_estimate(&fixture.db, &volume);
    assert_eq!(estimate, None);
    assert!(load_cached_index(&fixture.db, &volume, estimate).is_none());
}

#[test]
fn missing_rows_or_sqlite_read_failure_leave_cache_unpublished() {
    let fixture = TestDb::new();
    let source = index();
    fixture.db.save_volume(&source, |_, _| {}).unwrap();
    let volume = volume();
    assert_eq!(matching_record_estimate(&fixture.db, &volume), Some(0));
    assert!(load_cached_index(&fixture.db, &volume, Some(0)).is_none());
    fixture.sql.execute("DROP TABLE file_records", []).unwrap();
    assert!(load_cached_index(&fixture.db, &volume, Some(1)).is_none());
}

#[test]
fn record_estimate_is_not_a_completeness_requirement_for_non_usn_cache() {
    let fixture = TestDb::new();
    let mut source = index();
    assert!(source.insert_record(20, "file.txt", 5, false, false));
    fixture.db.save_volume(&source, |_, _| {}).unwrap();
    fixture
        .sql
        .execute("UPDATE volume_state SET files_indexed = 10", [])
        .unwrap();
    let volume = volume();
    let estimate = matching_record_estimate(&fixture.db, &volume);
    assert_eq!(estimate, Some(10));
    // Non-USN publishes a startup snapshot while a full scan reconciles it.
    assert_eq!(
        load_cached_index(&fixture.db, &volume, estimate)
            .unwrap()
            .records
            .len(),
        1
    );
}
