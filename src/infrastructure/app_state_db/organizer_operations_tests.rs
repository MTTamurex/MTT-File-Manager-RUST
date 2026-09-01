use super::*;
use std::os::windows::ffi::OsStringExt;

#[test]
fn operation_ids_continue_after_database_reopen() {
    let state_dir = tempfile::tempdir().expect("state directory");
    let source = PathBuf::from(r"C:\source\report.txt");
    let destination = PathBuf::from(r"C:\destination\report.txt");

    let first_id = {
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        db.start_organizer_operation(7, &source, &destination)
            .expect("start operation")
    };
    let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("reopen database");
    let second_id = db
        .start_organizer_operation(7, &source, &destination)
        .expect("start operation after reopen");

    assert!(second_id.get() > first_id.get());
    assert_eq!(
        db.get_organizer_operation(first_id)
            .expect("read first operation")
            .expect("first record")
            .status,
        OrganizerOperationStatus::Started
    );
}

#[test]
fn operation_lifecycle_survives_database_reopen() {
    let state_dir = tempfile::tempdir().expect("state directory");
    let source = PathBuf::from(r"C:\source\report.txt");
    let destination = PathBuf::from(r"C:\destination\report.txt");

    let operation_id = {
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        let operation_id = db
            .start_organizer_operation(7, &source, &destination)
            .expect("start operation");
        db.finish_organizer_operation(operation_id, OrganizerOperationStatus::Completed, None)
            .expect("finish operation");
        operation_id
    };

    let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("reopen database");
    let record = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("persisted record");
    assert_eq!(record.rule_id, 7);
    assert_eq!(record.source_path, source);
    assert_eq!(record.destination_path, destination);
    assert_eq!(record.status, OrganizerOperationStatus::Completed);
    assert!(record.finished_at.is_some());
    assert_eq!(
        db.list_organizer_operations(1).expect("list operations"),
        vec![record]
    );
}

#[test]
fn terminal_operation_can_be_recorded_without_a_worker_start() {
    let db = AppStateDb::new_in_memory().expect("database");
    let source = PathBuf::from(r"C:\source\report.txt");
    let destination = PathBuf::from(r"C:\destination\report.txt");

    let operation_id = db
        .record_terminal_organizer_operation(
            3,
            &source,
            &destination,
            OrganizerOperationStatus::Skipped,
            Some("destination exists"),
        )
        .expect("record skipped operation");

    let record = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("terminal record");
    assert_eq!(record.status, OrganizerOperationStatus::Skipped);
    assert_eq!(record.started_at, record.finished_at.expect("finish time"));
    assert_eq!(record.error.as_deref(), Some("destination exists"));
}

#[test]
fn finishing_an_operation_twice_does_not_change_the_first_result() {
    let db = AppStateDb::new_in_memory().expect("database");
    let source = PathBuf::from(r"C:\source\report.txt");
    let destination = PathBuf::from(r"C:\destination\report.txt");
    let operation_id = db
        .start_organizer_operation(1, &source, &destination)
        .expect("start operation");

    db.finish_organizer_operation(
        operation_id,
        OrganizerOperationStatus::Failed,
        Some("first"),
    )
    .expect("finish operation");
    assert!(matches!(
        db.finish_organizer_operation(operation_id, OrganizerOperationStatus::Completed, None),
        Err(OrganizerOperationDbError::AlreadyFinalized(id)) if id == operation_id
    ));
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("record")
            .error
            .as_deref(),
        Some("first")
    );
}

#[test]
fn malformed_rows_are_reported_instead_of_silently_omitted() {
    let db = AppStateDb::new_in_memory().expect("database");
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "INSERT INTO organizer_operations
                (operation_id, rule_id, source_path, destination_path, status, started_at)
             VALUES ('99', 1, x'FF', 'destination', 'started', 1)",
            [],
        )
        .expect("insert malformed row");

    assert!(matches!(
        db.list_organizer_operations(10),
        Err(OrganizerOperationDbError::Database(
            rusqlite::Error::InvalidColumnType(..)
        ))
    ));
}

#[test]
fn windows_paths_round_trip_without_lossy_unicode_conversion() {
    let db = AppStateDb::new_in_memory().expect("database");
    let source = PathBuf::from(OsString::from_wide(&[
        b'C' as u16,
        b':' as u16,
        b'\\' as u16,
        0xD800,
    ]));
    let destination = PathBuf::from(OsString::from_wide(&[
        b'D' as u16,
        b':' as u16,
        b'\\' as u16,
        0xDFFF,
    ]));

    let operation_id = db
        .start_organizer_operation(1, &source, &destination)
        .expect("start operation");
    let record = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("operation record");

    assert_eq!(record.source_path, source);
    assert_eq!(record.destination_path, destination);
}

#[test]
fn legacy_operation_schema_is_extended_without_losing_history() {
    let state_dir = tempfile::tempdir().expect("state directory");
    let connection =
        rusqlite::Connection::open(state_dir.path().join("app_state.db")).expect("legacy database");
    connection
        .execute_batch(
            "CREATE TABLE organizer_operations (
                operation_id TEXT PRIMARY KEY,
                rule_id INTEGER NOT NULL,
                source_path TEXT NOT NULL,
                destination_path TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                error TEXT
             );
             INSERT INTO organizer_operations VALUES
                ('41', 9, 'C:\\old.txt', 'D:\\old.txt', 'completed', 1, 2, NULL);",
        )
        .expect("legacy schema");
    drop(connection);

    let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("migrated database");
    let legacy_id = OrganizerOperationId::from_raw(41).expect("legacy ID");
    assert_eq!(
        db.get_organizer_operation(legacy_id)
            .expect("read legacy operation")
            .expect("legacy operation")
            .source_path,
        PathBuf::from(r"C:\old.txt")
    );
    let blobs_present = db
        .writer
        .lock()
        .expect("writer")
        .query_row(
            "SELECT source_path_bytes IS NOT NULL AND destination_path_bytes IS NOT NULL
             FROM organizer_operations WHERE operation_id = '41'",
            [],
            |row| row.get::<_, bool>(0),
        )
        .expect("legacy path blobs");
    assert!(blobs_present);
    assert_eq!(
        db.start_organizer_operation(9, Path::new(r"C:\new.txt"), Path::new(r"D:\new.txt"))
            .expect("new operation")
            .get(),
        42
    );
}

#[test]
fn history_uses_numeric_ids_to_break_timestamp_ties() {
    let db = AppStateDb::new_in_memory().expect("database");
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "UPDATE organizer_operation_sequence SET next_id = 9 WHERE singleton = 1",
            [],
        )
        .expect("set sequence");
    let source = Path::new(r"C:\source.txt");
    let destination = Path::new(r"D:\destination.txt");
    let id_nine = db
        .start_organizer_operation(1, source, destination)
        .expect("operation nine");
    let id_ten = db
        .start_organizer_operation(1, source, destination)
        .expect("operation ten");
    db.writer
        .lock()
        .expect("writer")
        .execute("UPDATE organizer_operations SET started_at = 1", [])
        .expect("equalize timestamps");

    let ids = db
        .list_organizer_operations(2)
        .expect("list operations")
        .into_iter()
        .map(|record| record.operation_id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![id_ten, id_nine]);
}
