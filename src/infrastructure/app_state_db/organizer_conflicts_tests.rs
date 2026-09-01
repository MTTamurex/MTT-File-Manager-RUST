use super::*;
use crate::infrastructure::windows::shell_operations::organizer_file_snapshot;

#[test]
fn preflight_conflict_is_atomic_and_survives_database_reopen() {
    let state_dir = tempfile::tempdir().expect("state directory");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");

    let (operation_id, conflict_id) = {
        let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("database");
        match db
            .record_terminal_organizer_conflict(
                7,
                &source,
                &destination,
                source_snapshot,
                Some(destination_snapshot),
            )
            .expect("record conflict")
        {
            OrganizerConflictRegistration::Created {
                operation_id,
                conflict_id,
            } => (operation_id, conflict_id),
            _ => panic!("expected a new conflict"),
        }
    };

    let db = AppStateDb::new(state_dir.path().to_path_buf()).expect("reopen database");
    let operation = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("operation");
    let conflict = db
        .get_organizer_conflict(conflict_id)
        .expect("read conflict")
        .expect("conflict");

    assert_eq!(operation.conflict_id, Some(conflict_id));
    assert_eq!(conflict.operation_id, operation_id);
    assert_eq!(conflict.source_path, source);
    assert_eq!(conflict.destination_path, destination);
    assert_eq!(conflict.source_snapshot, source_snapshot);
    assert_eq!(conflict.destination_snapshot, Some(destination_snapshot));
    assert_eq!(conflict.status, OrganizerConflictStatus::Pending);

    assert_eq!(
        db.record_terminal_organizer_conflict(
            7,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("reuse conflict"),
        OrganizerConflictRegistration::Existing {
            operation_id,
            conflict_id,
        }
    );
}

#[test]
fn worker_conflict_creation_is_idempotent_and_links_the_operation() {
    let db = AppStateDb::new_in_memory().expect("database");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");
    let operation_id = db
        .start_organizer_operation(3, &source, &destination)
        .expect("start operation");

    let first = db
        .create_organizer_conflict(
            operation_id,
            3,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("create conflict");
    let second = db
        .create_organizer_conflict(
            operation_id,
            3,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("reuse conflict");

    let first_conflict_id = match first {
        OrganizerConflictRegistration::Created { conflict_id, .. } => conflict_id,
        _ => panic!("expected a new conflict"),
    };
    assert_eq!(
        second,
        OrganizerConflictRegistration::Existing {
            operation_id,
            conflict_id: first_conflict_id,
        }
    );
    assert_eq!(
        db.list_organizer_conflicts(10)
            .expect("list conflicts")
            .len(),
        1
    );
    assert_eq!(
        db.list_pending_organizer_conflicts(10)
            .expect("list pending conflicts")
            .len(),
        1
    );
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation")
            .conflict_id,
        Some(first_conflict_id)
    );
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation")
            .status,
        crate::domain::organizer_operation::OrganizerOperationStatus::Skipped
    );
}

#[test]
fn finishing_conflict_preserves_the_first_terminal_state() {
    let db = AppStateDb::new_in_memory().expect("database");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");
    let operation_id = db
        .start_organizer_operation(3, &source, &destination)
        .expect("start operation");
    let conflict_id = match db
        .create_organizer_conflict(
            operation_id,
            3,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("create conflict")
    {
        OrganizerConflictRegistration::Created { conflict_id, .. } => conflict_id,
        _ => panic!("expected a new conflict"),
    };

    db.finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Resolved)
        .expect("finish conflict");
    assert!(db
        .list_pending_organizer_conflicts(10)
        .expect("list pending conflicts")
        .is_empty());
    assert!(matches!(
        db.finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Cancelled),
        Err(OrganizerConflictDbError::AlreadyFinalized(id)) if id == conflict_id
    ));
    assert_eq!(
        db.get_organizer_conflict(conflict_id)
            .expect("read conflict")
            .expect("conflict")
            .status,
        OrganizerConflictStatus::Resolved
    );
}

#[test]
fn malformed_conflict_snapshots_are_reported() {
    let db = AppStateDb::new_in_memory().expect("database");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let operation_id = db
        .start_organizer_operation(3, &source, &destination)
        .expect("start operation");

    db.writer
        .lock()
        .expect("writer")
        .execute(
            "INSERT INTO organizer_conflicts
                (conflict_id, operation_id, rule_id, source_path, destination_path,
                 source_snapshot, created_at, last_checked_at, status)
             VALUES ('1', ?1, 3, 'source', 'destination', x'FF', 1, 1, 'pending')",
            rusqlite::params![operation_id.to_string()],
        )
        .expect("insert malformed conflict");

    assert!(matches!(
        db.list_organizer_conflicts(10),
        Err(OrganizerConflictDbError::Database(
            rusqlite::Error::FromSqlConversionFailure(..)
        ))
    ));
}

#[test]
fn cancelled_identity_is_suppressed_until_a_snapshot_changes() {
    let db = AppStateDb::new_in_memory().expect("database");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");
    let conflict_id = match db
        .record_terminal_organizer_conflict(
            7,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("create conflict")
    {
        OrganizerConflictRegistration::Created { conflict_id, .. } => conflict_id,
        _ => panic!("expected a new conflict"),
    };
    db.finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Cancelled)
        .expect("cancel conflict");

    let suppressed_operation = db
        .start_organizer_operation(7, &source, &destination)
        .expect("start suppressed operation");
    assert_eq!(
        db.create_organizer_conflict(
            suppressed_operation,
            7,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("suppress worker conflict"),
        OrganizerConflictRegistration::Suppressed
    );
    let suppressed_operation = db
        .get_organizer_operation(suppressed_operation)
        .expect("load suppressed operation")
        .expect("suppressed operation exists");
    assert_eq!(
        suppressed_operation.status,
        crate::domain::organizer_operation::OrganizerOperationStatus::Cancelled
    );
    assert_eq!(suppressed_operation.conflict_id, Some(conflict_id));

    assert_eq!(
        db.record_terminal_organizer_conflict(
            7,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("suppress conflict"),
        OrganizerConflictRegistration::Suppressed
    );
    let uppercase_source = PathBuf::from(source.to_string_lossy().to_uppercase());
    let uppercase_destination = PathBuf::from(destination.to_string_lossy().to_uppercase());
    assert_eq!(
        db.record_terminal_organizer_conflict(
            7,
            &uppercase_source,
            &uppercase_destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("suppress case-only path variant"),
        OrganizerConflictRegistration::Suppressed
    );

    std::fs::write(&source, b"changed source").expect("change source");
    let changed_snapshot = organizer_file_snapshot(&source).expect("changed snapshot");
    assert!(matches!(
        db.record_terminal_organizer_conflict(
            7,
            &source,
            &destination,
            changed_snapshot,
            Some(destination_snapshot),
        )
        .expect("create changed conflict"),
        OrganizerConflictRegistration::Created { .. }
    ));
    assert_eq!(
        db.list_pending_organizer_conflicts(10)
            .expect("pending conflicts")
            .len(),
        1
    );
}

#[test]
fn worker_reuses_pending_identity_and_finalizes_the_new_operation_atomically() {
    let db = AppStateDb::new_in_memory().expect("database");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("destination.txt");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    let source_snapshot = organizer_file_snapshot(&source).expect("source snapshot");
    let destination_snapshot = organizer_file_snapshot(&destination).expect("destination snapshot");
    let conflict_id = match db
        .record_terminal_organizer_conflict(
            7,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("create conflict")
    {
        OrganizerConflictRegistration::Created { conflict_id, .. } => conflict_id,
        _ => panic!("expected a new conflict"),
    };
    let operation_id = db
        .start_organizer_operation(7, &source, &destination)
        .expect("start worker operation");

    assert!(matches!(
        db.create_organizer_conflict(
            operation_id,
            7,
            &source,
            &destination,
            source_snapshot,
            Some(destination_snapshot),
        )
        .expect("reuse conflict"),
        OrganizerConflictRegistration::Existing {
            conflict_id: existing,
            ..
        } if existing == conflict_id
    ));
    let operation = db
        .get_organizer_operation(operation_id)
        .expect("read operation")
        .expect("operation");
    assert_eq!(
        operation.status,
        crate::domain::organizer_operation::OrganizerOperationStatus::Skipped
    );
    assert_eq!(operation.conflict_id, Some(conflict_id));
}

fn pending_conflict(
    rule_id: i64,
) -> (
    AppStateDb,
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    OrganizerConflictId,
) {
    let db = AppStateDb::new_in_memory().expect("database");
    let files_dir = tempfile::tempdir().expect("files directory");
    let source = files_dir.path().join("source.txt");
    let destination = files_dir.path().join("organized").join("source.txt");
    std::fs::create_dir_all(destination.parent().expect("destination parent"))
        .expect("create destination directory");
    std::fs::write(&source, b"source").expect("create source");
    std::fs::write(&destination, b"destination").expect("create destination");
    let registration = db
        .record_terminal_organizer_conflict(
            rule_id,
            &source,
            &destination,
            organizer_file_snapshot(&source).expect("source snapshot"),
            Some(organizer_file_snapshot(&destination).expect("destination snapshot")),
        )
        .expect("record conflict");
    let conflict_id = match registration {
        OrganizerConflictRegistration::Created { conflict_id, .. } => conflict_id,
        _ => panic!("expected a new conflict"),
    };
    (db, files_dir, source, destination, conflict_id)
}

#[test]
fn claimed_conflict_cannot_be_cancelled_concurrently() {
    let (db, files_dir, _source, _destination, conflict_id) = pending_conflict(11);
    let target = files_dir.path().join("source (1).txt");
    db.claim_organizer_conflict_resolution(conflict_id, true, &target)
        .expect("claim conflict");
    db.reconcile_organizer_conflict_resolutions()
        .expect("active claims are left alone");

    let error = db
        .finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Cancelled)
        .expect_err("cancel must be blocked");

    assert!(matches!(
        error,
        OrganizerConflictDbError::ResolutionInProgress(id) if id == conflict_id
    ));
}

#[test]
fn reconciliation_releases_a_claim_when_the_move_did_not_happen() {
    let (db, files_dir, _source, _destination, conflict_id) = pending_conflict(12);
    let target = files_dir.path().join("source (1).txt");
    db.claim_organizer_conflict_resolution(conflict_id, true, &target)
        .expect("claim conflict");

    db.reconcile_owned_organizer_conflict_resolution(conflict_id)
        .expect("reconcile conflict");
    db.finish_organizer_conflict(conflict_id, OrganizerConflictStatus::Cancelled)
        .expect("released conflict can be cancelled");

    let conflict = db
        .get_organizer_conflict(conflict_id)
        .expect("load conflict")
        .expect("conflict exists");
    assert_eq!(conflict.status, OrganizerConflictStatus::Cancelled);
}

#[test]
fn reconciliation_finalizes_a_move_completed_before_database_commit() {
    let (db, files_dir, source, _destination, conflict_id) = pending_conflict(13);
    let target = files_dir.path().join("source (1).txt");
    db.claim_organizer_conflict_resolution(conflict_id, true, &target)
        .expect("claim conflict");
    std::fs::rename(&source, &target).expect("simulate completed move");

    db.reconcile_owned_organizer_conflict_resolution(conflict_id)
        .expect("reconcile conflict");

    let conflict = db
        .get_organizer_conflict(conflict_id)
        .expect("load conflict")
        .expect("conflict exists");
    assert_eq!(conflict.status, OrganizerConflictStatus::Resolved);
}

#[test]
fn startup_reconciliation_recovers_a_claim_from_a_stopped_owner() {
    let (db, files_dir, source, _destination, conflict_id) = pending_conflict(16);
    let target = files_dir.path().join("source (1).txt");
    db.claim_organizer_conflict_resolution(conflict_id, true, &target)
        .expect("claim conflict");
    db.writer
        .lock()
        .expect("writer")
        .execute(
            "UPDATE organizer_conflict_resolutions SET owner_id = '0:0' WHERE conflict_id = ?1",
            rusqlite::params![conflict_id.to_string()],
        )
        .expect("simulate stopped owner");
    std::fs::rename(&source, &target).expect("simulate completed move");

    db.reconcile_organizer_conflict_resolutions()
        .expect("startup reconciliation");

    let conflict = db
        .get_organizer_conflict(conflict_id)
        .expect("load conflict")
        .expect("conflict exists");
    assert_eq!(conflict.status, OrganizerConflictStatus::Resolved);
}

#[test]
fn reconciliation_rejects_an_unrelated_file_at_the_claimed_target() {
    let (db, files_dir, source, _destination, conflict_id) = pending_conflict(14);
    let target = files_dir.path().join("source (1).txt");
    db.claim_organizer_conflict_resolution(conflict_id, true, &target)
        .expect("claim conflict");
    std::fs::remove_file(&source).expect("remove original");
    std::fs::write(&target, b"unrelated").expect("create unrelated target");

    db.reconcile_owned_organizer_conflict_resolution(conflict_id)
        .expect("reconcile conflict");

    let conflict = db
        .get_organizer_conflict(conflict_id)
        .expect("load conflict")
        .expect("conflict exists");
    assert_eq!(conflict.status, OrganizerConflictStatus::Obsolete);
}

#[test]
fn completed_move_obsoletes_a_previous_conflict_for_the_same_paths() {
    let (db, _files_dir, source, destination, conflict_id) = pending_conflict(15);
    let operation_id = db
        .start_organizer_operation(15, &source, &destination)
        .expect("start replacement operation");

    db.finish_organizer_operation(
        operation_id,
        crate::domain::organizer_operation::OrganizerOperationStatus::Completed,
        None,
    )
    .expect("finish replacement move");

    let conflict = db
        .get_organizer_conflict(conflict_id)
        .expect("load conflict")
        .expect("conflict exists");
    assert_eq!(conflict.status, OrganizerConflictStatus::Obsolete);
}
