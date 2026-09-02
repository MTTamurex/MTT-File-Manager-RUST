use super::*;
use crate::domain::organizer_operation::OrganizerOperationStatus;
use crate::domain::organizer_rule::OrganizerConflictPolicy;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::workers::file_operation_worker::OrganizerUndoExemptionRegistry;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;

#[test]
fn auto_rename_suffix_stays_within_the_windows_component_limit() {
    use std::os::windows::ffi::OsStrExt;

    let base_name = format!("{}.txt", "a".repeat(251));
    let renamed = suffixed_file_name(std::ffi::OsStr::new(&base_name), 1);
    let units: Vec<u16> = renamed.encode_wide().collect();

    assert!(units.len() <= 255);
    assert!(renamed.to_string_lossy().ends_with(" (1).txt"));
}

fn run_move(
    source: PathBuf,
    destination_folder: &Path,
    policy: OrganizerConflictPolicy,
) -> (FileOperationResult, AppStateDb, PathBuf) {
    let file_name = source.file_name().expect("source file name");
    let destination = destination_folder.join(file_name);
    let snapshot = shell_operations::organizer_file_snapshot(&source).expect("source snapshot");
    let app_state_db = AppStateDb::new_in_memory().expect("database");
    let operation_id = app_state_db
        .start_organizer_operation_with_snapshot(7, &source, &destination, snapshot)
        .expect("start operation");
    let activation = Arc::new(AtomicBool::new(true));
    let shutdown = Arc::new(AtomicBool::new(false));
    let undo_exemptions = OrganizerUndoExemptionRegistry::default();
    let (result_sender, result_receiver) = mpsc::channel();

    handle_organizer_move(
        source,
        destination_folder.to_path_buf(),
        OrganizerMoveContext {
            operation: (operation_id, 7),
            lifecycle: (activation, shutdown),
            expected_snapshot: snapshot,
            conflict_policy: policy,
            is_undo: false,
            undo_exemptions: &undo_exemptions,
            app_state_db: &app_state_db,
            result_sender: &result_sender,
        },
    );

    (
        result_receiver.recv().expect("move result"),
        app_state_db,
        destination,
    )
}

#[test]
fn skip_policy_finishes_without_creating_a_conflict() {
    let source_parent = tempfile::tempdir().expect("source parent");
    let destination_parent = tempfile::tempdir().expect("destination parent");
    let source = source_parent.path().join("report.pdf");
    let destination = destination_parent.path().join("report.pdf");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");

    let (result, app_state_db, _) = run_move(
        source.clone(),
        destination_parent.path(),
        OrganizerConflictPolicy::Skip,
    );

    let operation_id = match result {
        FileOperationResult::OrganizerMoveSkipped {
            operation_id,
            path,
            destination: result_destination,
            conflict_id: None,
            ..
        } => {
            assert_eq!(path, source);
            assert_eq!(result_destination, destination);
            operation_id
        }
        _ => panic!("expected skipped move"),
    };
    assert_eq!(std::fs::read(&source).expect("source remains"), b"source");
    assert_eq!(
        std::fs::read(&destination).expect("destination remains"),
        b"destination"
    );
    assert_eq!(
        app_state_db
            .get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation record")
            .status,
        OrganizerOperationStatus::Skipped
    );
    assert!(app_state_db
        .list_pending_organizer_conflicts(10)
        .expect("read conflicts")
        .is_empty());
}

#[test]
fn auto_rename_policy_uses_the_first_free_name_without_replacing_files() {
    let source_parent = tempfile::tempdir().expect("source parent");
    let destination_parent = tempfile::tempdir().expect("destination parent");
    let source = source_parent.path().join("report.pdf");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(destination_parent.path().join("report.pdf"), b"existing")
        .expect("existing destination");
    std::fs::write(
        destination_parent.path().join("report (1).pdf"),
        b"existing one",
    )
    .expect("first renamed destination");

    let (result, app_state_db, planned_destination) = run_move(
        source.clone(),
        destination_parent.path(),
        OrganizerConflictPolicy::AutoRenameSource,
    );
    let renamed_destination = destination_parent.path().join("report (2).pdf");
    let operation_id = match result {
        FileOperationResult::OrganizerMoveCompleted {
            operation_id,
            source_path,
            moved_dest,
            ..
        } => {
            assert_eq!(source_path, source);
            assert_eq!(moved_dest, renamed_destination);
            operation_id
        }
        _ => panic!("expected auto-renamed move"),
    };
    assert!(!source.exists());
    assert_eq!(
        std::fs::read(&planned_destination).expect("original destination remains"),
        b"existing"
    );
    assert_eq!(
        std::fs::read(destination_parent.path().join("report (1).pdf"))
            .expect("first renamed destination remains"),
        b"existing one"
    );
    assert_eq!(
        std::fs::read(&renamed_destination).expect("renamed destination"),
        b"source"
    );
    assert_eq!(
        app_state_db
            .get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation record")
            .effective_destination_path,
        Some(renamed_destination)
    );
}

#[test]
fn conflict_folder_policy_moves_the_source_without_replacing_the_destination() {
    let source_parent = tempfile::tempdir().expect("source parent");
    let destination_parent = tempfile::tempdir().expect("destination parent");
    let conflict_parent = tempfile::tempdir().expect("conflict parent");
    let source = source_parent.path().join("report.pdf");
    let destination = destination_parent.path().join("report.pdf");
    let conflict_destination = conflict_parent.path().join("report.pdf");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");

    let (result, _, _) = run_move(
        source.clone(),
        destination_parent.path(),
        OrganizerConflictPolicy::MoveToConflictFolder(conflict_parent.path().to_path_buf()),
    );

    match result {
        FileOperationResult::OrganizerMoveCompleted { moved_dest, .. } => {
            assert_eq!(moved_dest, conflict_destination);
        }
        _ => panic!("expected conflict-folder move"),
    }
    assert!(!source.exists());
    assert_eq!(
        std::fs::read(&destination).expect("destination remains"),
        b"destination"
    );
    assert_eq!(
        std::fs::read(&conflict_destination).expect("conflict destination"),
        b"source"
    );
}

#[test]
fn occupied_conflict_folder_target_is_reported_without_replacing_either_file() {
    let source_parent = tempfile::tempdir().expect("source parent");
    let destination_parent = tempfile::tempdir().expect("destination parent");
    let conflict_parent = tempfile::tempdir().expect("conflict parent");
    let source = source_parent.path().join("report.pdf");
    let destination = destination_parent.path().join("report.pdf");
    let conflict_destination = conflict_parent.path().join("report.pdf");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::write(&destination, b"destination").expect("destination file");
    std::fs::write(&conflict_destination, b"conflict").expect("conflict file");

    let (result, app_state_db, _) = run_move(
        source.clone(),
        destination_parent.path(),
        OrganizerConflictPolicy::MoveToConflictFolder(conflict_parent.path().to_path_buf()),
    );

    let (operation_id, conflict_id) = match result {
        FileOperationResult::OrganizerMoveSkipped {
            operation_id,
            path,
            destination: result_destination,
            conflict_id: Some(conflict_id),
            ..
        } => {
            assert_eq!(path, source);
            assert_eq!(result_destination, conflict_destination);
            (operation_id, conflict_id)
        }
        _ => panic!("expected conflict-folder collision"),
    };
    assert!(app_state_db
        .get_organizer_conflict(conflict_id)
        .expect("read conflict")
        .is_some());
    assert_eq!(
        app_state_db
            .get_organizer_operation(operation_id)
            .expect("read operation")
            .expect("operation record")
            .effective_destination_path,
        Some(conflict_destination.clone())
    );
    assert_eq!(std::fs::read(&source).expect("source remains"), b"source");
    assert_eq!(
        std::fs::read(&destination).expect("destination remains"),
        b"destination"
    );
    assert_eq!(
        std::fs::read(&conflict_destination).expect("conflict destination remains"),
        b"conflict"
    );
}
