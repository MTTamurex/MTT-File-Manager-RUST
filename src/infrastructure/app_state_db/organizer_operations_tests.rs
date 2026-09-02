use super::*;
use crate::domain::organizer_operation::OrganizerOperationType;
use crate::infrastructure::app_state_db::{AppStateDb, OrganizerConflictRegistration};
use crate::infrastructure::windows::shell_operations::{
    move_organizer_file_without_replace, organizer_file_snapshot,
};
use std::os::windows::ffi::OsStringExt;

fn mark_operation_owner_dead(db: &AppStateDb, operation_id: OrganizerOperationId) {
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "UPDATE organizer_operations SET owner_id = '0:0' WHERE operation_id = ?1",
            rusqlite::params![operation_id.to_string()],
        )
        .expect("mark operation owner dead");
}

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
    assert_eq!(record.rule_id, Some(7));
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
fn nullable_rule_migration_preserves_legacy_conflict_foreign_keys() {
    let state_dir = tempfile::tempdir().expect("state directory");
    let connection =
        rusqlite::Connection::open(state_dir.path().join("app_state.db")).expect("legacy database");
    connection
        .execute_batch(
            r#"PRAGMA foreign_keys = ON;
             CREATE TABLE organizer_operations (
                 operation_id TEXT PRIMARY KEY,
                 rule_id INTEGER NOT NULL,
                 source_path TEXT NOT NULL,
                 destination_path TEXT NOT NULL,
                 status TEXT NOT NULL,
                 started_at INTEGER NOT NULL,
                 finished_at INTEGER,
                 error TEXT,
                 conflict_id TEXT
             );
             CREATE TABLE organizer_conflicts (
                 conflict_id TEXT PRIMARY KEY,
                 operation_id TEXT NOT NULL UNIQUE,
                 rule_id INTEGER NOT NULL,
                 source_path TEXT NOT NULL,
                 destination_path TEXT NOT NULL,
                 source_snapshot BLOB NOT NULL,
                 destination_snapshot BLOB,
                 created_at INTEGER NOT NULL,
                 last_checked_at INTEGER NOT NULL,
                 status TEXT NOT NULL,
                 FOREIGN KEY (operation_id) REFERENCES organizer_operations(operation_id)
             );
             INSERT INTO organizer_operations VALUES
                 ('41', 9, 'C:\old.txt', 'D:\old.txt', 'skipped', 1, 2, NULL, '7');
             INSERT INTO organizer_conflicts VALUES
                 ('7', '41', 9, 'C:\old.txt', 'D:\old.txt', zeroblob(36), NULL,
                  1, 2, 'pending');"#,
        )
        .expect("legacy schema with conflict");
    drop(connection);

    let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("migrated database");
    let conflict_id =
        crate::domain::organizer_conflict::OrganizerConflictId::from_raw(7).expect("conflict ID");
    assert_eq!(
        db.get_organizer_conflict(conflict_id)
            .expect("read conflict")
            .expect("legacy conflict")
            .operation_id,
        OrganizerOperationId::from_raw(41).expect("operation ID")
    );
    assert_eq!(
        db.writer
            .lock()
            .expect("writer")
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()
            .expect("foreign key check"),
        None
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

#[test]
fn completed_operations_store_identity_and_can_create_one_undo_attempt() {
    let root = tempfile::tempdir().expect("test directory");
    let source_folder = root.path().join("source");
    let destination_folder = root.path().join("destination");
    std::fs::create_dir(&source_folder).expect("source folder");
    std::fs::create_dir(&destination_folder).expect("destination folder");
    let source = source_folder.join("report.txt");
    let destination = destination_folder.join("report.txt");
    std::fs::write(&source, b"contents").expect("source file");
    let before = organizer_file_snapshot(&source).expect("source snapshot");
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation_with_snapshot(7, &source, &destination, before)
        .expect("start operation");

    move_organizer_file_without_replace(&source, &destination, before).expect("move file");
    let after = organizer_file_snapshot(&destination).expect("destination snapshot");
    db.finish_organizer_operation_with_metadata(
        operation_id,
        OrganizerOperationStatus::Completed,
        None,
        Some(&source),
        Some(&destination),
        Some(after),
    )
    .expect("finish operation");

    let record = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("record");
    assert_eq!(record.operation_type, OrganizerOperationType::Move);
    assert_eq!(record.source_snapshot_before, Some(before));
    assert_eq!(record.destination_snapshot_after, Some(after));

    let undo_id = db
        .create_undo_organizer_operation(operation_id, &destination, &source, after)
        .expect("create undo");
    let undo = db
        .get_organizer_operation(undo_id)
        .expect("read undo")
        .expect("undo record");
    assert_eq!(undo.operation_type, OrganizerOperationType::Undo);
    assert_eq!(undo.rule_id, Some(7));
    assert_eq!(undo.original_operation_id, Some(operation_id));
    assert_eq!(undo.source_snapshot_before, Some(after));
    assert!(matches!(
        db.create_undo_organizer_operation(operation_id, &destination, &source, after),
        Err(OrganizerOperationDbError::UndoUnavailable(id)) if id == operation_id
    ));

    let restored_snapshot = move_organizer_file_without_replace(&destination, &source, after)
        .expect("execute undo move");
    db.finish_organizer_operation_with_metadata(
        undo_id,
        OrganizerOperationStatus::Completed,
        None,
        Some(&destination),
        Some(&source),
        Some(restored_snapshot),
    )
    .expect("finish undo");
    assert!(db
        .get_organizer_operation(operation_id)
        .expect("read original after undo")
        .expect("original operation")
        .undone_at
        .is_some());
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "DELETE FROM organizer_operations WHERE operation_id = ?1",
            rusqlite::params![undo_id.to_string()],
        )
        .expect("remove undo history child");
    assert!(matches!(
        db.create_undo_organizer_operation(operation_id, &destination, &source, after),
        Err(OrganizerOperationDbError::UndoUnavailable(id)) if id == operation_id
    ));
}

#[test]
fn retry_attempts_are_linked_to_terminal_operations() {
    let db = AppStateDb::new_in_memory().expect("database");
    let source = PathBuf::from(r"C:\source\report.txt");
    let destination = PathBuf::from(r"C:\destination\report.txt");
    let snapshot = OrganizerFileSnapshot::from_bytes(&[0; 36]).expect("snapshot fixture");
    let original = db
        .start_organizer_operation_with_snapshot(3, &source, &destination, snapshot)
        .expect("start original");
    db.finish_organizer_operation(original, OrganizerOperationStatus::Failed, Some("failed"))
        .expect("finish original");

    let retry = db
        .create_retry_organizer_operation(original, 3, &source, &destination, snapshot)
        .expect("create retry");
    let record = db
        .get_organizer_operation(retry)
        .expect("read retry")
        .expect("retry record");
    assert_eq!(record.operation_type, OrganizerOperationType::Retry);
    assert_eq!(record.original_operation_id, Some(original));
    assert_eq!(record.status, OrganizerOperationStatus::Started);

    let unrelated_source = PathBuf::from(r"C:\source\other.txt");
    assert!(matches!(
        db.create_retry_organizer_operation(
            original,
            3,
            &unrelated_source,
            &destination,
            snapshot,
        ),
        Err(OrganizerOperationDbError::RetryUnavailable(id)) if id == original
    ));
}

#[test]
fn interrupted_started_operations_are_terminalized() {
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation(
            1,
            Path::new(r"C:\source\report.txt"),
            Path::new(r"C:\destination\report.txt"),
        )
        .expect("start operation");
    mark_operation_owner_dead(&db, operation_id);

    assert_eq!(
        db.reconcile_started_organizer_operations()
            .expect("reconcile operations"),
        1
    );
    let operation = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("operation record");
    assert_eq!(operation.status, OrganizerOperationStatus::Failed);
    assert!(operation.finished_at.is_some());
    assert!(operation.error.is_some());
}

#[test]
fn reconciliation_preserves_operations_owned_by_the_current_process() {
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation(
            1,
            Path::new(r"C:\source\report.txt"),
            Path::new(r"C:\destination\report.txt"),
        )
        .expect("start operation");

    assert_eq!(
        db.reconcile_started_organizer_operations()
            .expect("reconcile operations"),
        0
    );
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation record")
            .status,
        OrganizerOperationStatus::Started
    );
}

#[test]
fn journaled_moves_are_recovered_as_completed() {
    let root = tempfile::tempdir().expect("test directory");
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, b"contents").expect("create source");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation_with_snapshot(1, &source, &destination, source_snapshot)
        .expect("start operation");
    let destination_snapshot =
        move_organizer_file_without_replace(&source, &destination, source_snapshot)
            .expect("move file");
    db.record_organizer_operation_completion(
        operation_id,
        &source,
        &destination,
        destination_snapshot,
    )
    .expect("journal completion");
    mark_operation_owner_dead(&db, operation_id);

    assert_eq!(
        db.reconcile_started_organizer_operations()
            .expect("reconcile operations"),
        1
    );
    let operation = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("operation record");
    assert_eq!(operation.status, OrganizerOperationStatus::Completed);
    assert_eq!(
        operation.effective_source_path.as_deref(),
        Some(source.as_path())
    );
    assert_eq!(
        operation.effective_destination_path.as_deref(),
        Some(destination.as_path())
    );
    assert_eq!(
        operation.destination_snapshot_after,
        Some(destination_snapshot)
    );
}

#[test]
fn targeted_completion_recovery_leaves_other_operations_started() {
    let root = tempfile::tempdir().expect("test directory");
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, b"contents").expect("create source");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation_with_snapshot(1, &source, &destination, source_snapshot)
        .expect("start operation");
    let unrelated_operation_id = db
        .start_organizer_operation(
            1,
            Path::new(r"C:\source\unrelated.txt"),
            Path::new(r"C:\destination\unrelated.txt"),
        )
        .expect("start unrelated operation");
    let destination_snapshot =
        move_organizer_file_without_replace(&source, &destination, source_snapshot)
            .expect("move file");
    db.record_organizer_operation_completion(
        operation_id,
        &source,
        &destination,
        destination_snapshot,
    )
    .expect("journal completion");

    assert!(db
        .reconcile_organizer_operation_completion(operation_id)
        .expect("recover operation"));
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation record")
            .status,
        OrganizerOperationStatus::Completed
    );
    assert_eq!(
        db.get_organizer_operation(unrelated_operation_id)
            .expect("read unrelated operation")
            .expect("unrelated operation record")
            .status,
        OrganizerOperationStatus::Started
    );
}

#[test]
fn incomplete_cross_volume_publish_is_rolled_back() {
    let root = tempfile::tempdir().expect("test directory");
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, b"contents").expect("create source");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation_with_snapshot(1, &source, &destination, source_snapshot)
        .expect("start operation");
    std::fs::write(&destination, b"copied contents").expect("publish partial destination");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");
    db.record_organizer_operation_completion(
        operation_id,
        &source,
        &destination,
        destination_snapshot,
    )
    .expect("journal completion");

    assert!(!db
        .reconcile_organizer_operation_completion(operation_id)
        .expect("reconcile partial move"));
    assert!(source.exists());
    assert!(!destination.exists());
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation record")
            .status,
        OrganizerOperationStatus::Started
    );
}

#[test]
fn conflict_finalization_removes_a_pre_publish_journal() {
    let root = tempfile::tempdir().expect("test directory");
    let source = root.path().join("source.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("create source");
    std::fs::write(&destination, b"destination").expect("create destination");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation_with_snapshot(1, &source, &destination, source_snapshot)
        .expect("start operation");
    db.record_organizer_operation_completion(operation_id, &source, &destination, source_snapshot)
        .expect("record pre-publish journal");

    db.create_organizer_conflict(
        operation_id,
        1,
        &source,
        &destination,
        source_snapshot,
        Some(destination_snapshot),
    )
    .expect("create conflict");

    let journal_count = db
        .writer
        .lock()
        .expect("writer")
        .query_row(
            "SELECT COUNT(*) FROM organizer_operation_completions WHERE operation_id = ?1",
            rusqlite::params![operation_id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .expect("count journals");
    assert_eq!(journal_count, 0);
}

#[test]
fn retention_keeps_pending_conflict_operations() {
    let root = tempfile::tempdir().expect("test directory");
    let source_folder = root.path().join("source");
    let destination_folder = root.path().join("destination");
    std::fs::create_dir(&source_folder).expect("source folder");
    std::fs::create_dir(&destination_folder).expect("destination folder");
    let source = source_folder.join("report.txt");
    let destination = destination_folder.join("report.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let db = AppStateDb::new_in_memory().expect("database");
    let operation_id = db
        .start_organizer_operation_with_snapshot(
            1,
            &source,
            &destination,
            organizer_file_snapshot(&source).expect("source snapshot"),
        )
        .expect("start operation");
    let conflict_id = match db
        .create_organizer_conflict(
            operation_id,
            1,
            &source,
            &destination,
            organizer_file_snapshot(&source).expect("source snapshot"),
            Some(organizer_file_snapshot(&destination).expect("destination snapshot")),
        )
        .expect("create conflict")
    {
        OrganizerConflictRegistration::Created { conflict_id, .. } => conflict_id,
        _ => panic!("expected created conflict"),
    };
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "UPDATE organizer_operations SET created_at = 1, started_at = 1, finished_at = 1",
            [],
        )
        .expect("age operation");
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "UPDATE organizer_conflicts SET created_at = 1, last_checked_at = 1",
            [],
        )
        .expect("age conflict");

    assert_eq!(
        db.retain_organizer_history(1, 50).expect("retain history"),
        0
    );
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("pending operation")
            .status,
        OrganizerOperationStatus::Skipped
    );
    assert_eq!(
        db.get_organizer_conflict(conflict_id)
            .expect("read conflict")
            .expect("pending conflict")
            .status,
        crate::domain::organizer_conflict::OrganizerConflictStatus::Pending
    );
}

#[test]
fn retention_removes_old_terminal_operations_and_resolved_conflicts() {
    let db = AppStateDb::new_in_memory().expect("database");
    let source = PathBuf::from(r"C:\source\report.txt");
    let destination = PathBuf::from(r"C:\destination\report.txt");
    let operation_id = db
        .record_terminal_organizer_operation(
            1,
            &source,
            &destination,
            OrganizerOperationStatus::Skipped,
            Some("old"),
        )
        .expect("record operation");
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "UPDATE organizer_operations SET created_at = 1, started_at = 1, finished_at = 1",
            [],
        )
        .expect("age operation");

    assert_eq!(
        db.retain_organizer_history(1, 50).expect("retain history"),
        1
    );
    assert!(db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .is_none());
}

#[test]
fn invalid_retention_preference_is_reported_instead_of_defaulting() {
    let db = AppStateDb::new_in_memory().expect("database");
    db.set_preference("organizer_history_retention_days", "invalid")
        .expect("write invalid fixture");

    assert!(matches!(
        db.try_organizer_history_retention_days(),
        Err(OrganizerOperationDbError::InvalidRetention)
    ));
}
