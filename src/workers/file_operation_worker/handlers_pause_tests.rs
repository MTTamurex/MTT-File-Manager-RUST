use super::*;
use crate::domain::organizer_operation::OrganizerOperationStatus;
use crate::workers::file_operation_worker::OrganizerUndoExemptionRegistry;
use std::sync::{atomic::AtomicBool, mpsc, Arc};

fn run_move(
    db: &AppStateDb,
    source: &Path,
    destination_folder: &Path,
    is_undo: bool,
) -> (
    FileOperationResult,
    crate::domain::organizer_operation::OrganizerOperationId,
) {
    let destination = destination_folder.join(source.file_name().unwrap());
    let snapshot = shell_operations::organizer_file_snapshot(source).unwrap();
    let operation_id = db
        .start_organizer_operation_with_snapshot(1, source, &destination, snapshot)
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    handle_organizer_move(
        source.to_path_buf(),
        destination_folder.to_path_buf(),
        OrganizerMoveContext {
            operation: (operation_id, 1),
            lifecycle: (
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(false)),
            ),
            destination_path: None,
            expected_snapshot: snapshot,
            conflict_policy: OrganizerConflictPolicy::Ask,
            is_undo,
            undo_exemptions: &OrganizerUndoExemptionRegistry::default(),
            app_state_db: db,
            result_sender: &sender,
        },
    );
    (receiver.recv().unwrap(), operation_id)
}

#[test]
fn organizer_global_pause_blocks_queued_moves_but_not_explicit_undo() {
    for is_undo in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("file.txt");
        let destination_folder = root.path().join("destination");
        std::fs::write(&source, b"content").unwrap();
        std::fs::create_dir(&destination_folder).unwrap();
        let destination = destination_folder.join("file.txt");
        let db = AppStateDb::new_in_memory().unwrap();
        db.set_organizer_globally_paused(true).unwrap();
        let (result, operation_id) = run_move(&db, &source, &destination_folder, is_undo);
        let record = db.get_organizer_operation(operation_id).unwrap().unwrap();
        if is_undo {
            assert!(matches!(
                result,
                FileOperationResult::OrganizerMoveCompleted { .. }
            ));
            assert_eq!(record.status, OrganizerOperationStatus::Completed);
            assert!(!source.exists());
            assert_eq!(std::fs::read(&destination).unwrap(), b"content");
        } else {
            assert!(matches!(
                result,
                FileOperationResult::OrganizerMoveCancelled { .. }
            ));
            assert_eq!(record.status, OrganizerOperationStatus::Cancelled);
            assert_eq!(std::fs::read(&source).unwrap(), b"content");
            assert!(!destination.exists());
        }
    }
}

#[test]
fn organizer_global_pause_read_failure_prevents_starting_a_move() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("file.txt");
    let destination_folder = root.path().join("destination");
    std::fs::write(&source, b"content").unwrap();
    std::fs::create_dir(&destination_folder).unwrap();
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("organizer_globally_paused", "invalid")
        .unwrap();
    let (result, _) = run_move(&db, &source, &destination_folder, false);
    assert!(matches!(
        result,
        FileOperationResult::OrganizerMoveCancelled { .. }
    ));
    assert!(source.exists());
    assert!(!destination_folder.join("file.txt").exists());
}

#[test]
fn organizer_move_in_progress_finishes_when_global_pause_is_set_before_commit() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("file.txt");
    let next_source = root.path().join("next.txt");
    let destination_folder = root.path().join("destination");
    std::fs::create_dir(&destination_folder).unwrap();
    std::fs::write(&source, b"content").unwrap();
    std::fs::write(&next_source, b"next").unwrap();
    let db = AppStateDb::new_in_memory().unwrap();
    // The move journals its completion immediately before publishing the rename.
    // Set pause at that exact point without timing-dependent threads or sleeps.
    db.test_hold_writer()
        .execute_batch(
            "CREATE TRIGGER pause_during_move AFTER INSERT ON organizer_operation_completions
         BEGIN
             INSERT OR REPLACE INTO user_preferences (key, value)
             VALUES ('organizer_globally_paused', 'true');
         END;",
        )
        .unwrap();
    let (result, operation_id) = run_move(&db, &source, &destination_folder, false);
    assert!(matches!(
        result,
        FileOperationResult::OrganizerMoveCompleted { .. }
    ));
    assert!(db.organizer_globally_paused().unwrap());
    assert_eq!(
        db.get_organizer_operation(operation_id)
            .unwrap()
            .unwrap()
            .status,
        OrganizerOperationStatus::Completed
    );
    assert!(!source.exists());
    assert_eq!(
        std::fs::read(destination_folder.join("file.txt")).unwrap(),
        b"content"
    );
    let (result, _) = run_move(&db, &next_source, &destination_folder, false);
    assert!(matches!(
        result,
        FileOperationResult::OrganizerMoveCancelled { .. }
    ));
    assert!(next_source.exists());
    assert!(!destination_folder.join("next.txt").exists());
}
