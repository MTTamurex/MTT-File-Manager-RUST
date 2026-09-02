use super::*;
use crate::domain::organizer_operation::OrganizerOperationStatus;
use crate::domain::organizer_rule::{OrganizerConflictPolicy, OrganizerRule};
use crate::infrastructure::app_state_db::AppStateDb;

#[test]
fn skip_policy_records_a_terminal_skip_without_dispatching_a_worker_request() {
    let root = tempfile::tempdir().expect("root directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("source directory");
    std::fs::create_dir(&destination).expect("destination directory");
    let source_path = source.join("report.txt");
    let destination_path = destination.join("report.txt");
    std::fs::write(&source_path, b"source").expect("source file");
    std::fs::write(&destination_path, b"destination").expect("destination file");

    let rule = OrganizerRule::new_with_conflict_policy(
        1,
        source.clone(),
        destination.clone(),
        vec!["txt".to_string()],
        true,
        OrganizerConflictPolicy::Skip,
    )
    .expect("valid rule");
    let rules = vec![rule];
    let activations = activation_flags_for(&rules);
    let paused_rules = HashSet::new();
    let in_flight = OrganizerInFlightRegistry::default();
    let undo_exemptions = OrganizerUndoExemptionRegistry::default();
    let app_state_db = Arc::new(AppStateDb::new_in_memory().expect("database"));
    let shutdown = Arc::new(AtomicBool::new(false));
    let mut pending = HashMap::new();
    queue_matching_path(
        &rules,
        &activations,
        &paused_rules,
        source_path.clone(),
        &mut pending,
    );
    pending
        .get_mut(&source_path)
        .expect("source should be pending")
        .stable_since = Instant::now() - STABILITY_DELAY;

    let (operation_sender, operation_receiver) = crossbeam_channel::unbounded();
    let (event_sender, event_receiver) = std::sync::mpsc::channel();
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        (&in_flight, &undo_exemptions),
        &app_state_db,
        &shutdown,
    );

    assert!(pending.is_empty());
    assert!(operation_receiver.try_recv().is_err());
    let operation_id = match event_receiver.try_recv() {
        Ok(OrganizerEvent::OperationSkipped {
            operation_id,
            conflict_id: None,
            path,
            destination,
            ..
        }) => {
            assert_eq!(path, source_path);
            assert_eq!(destination, destination_path);
            operation_id
        }
        _ => panic!("expected terminal skip event"),
    };
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

    queue_matching_path(
        &rules,
        &activations,
        &paused_rules,
        source_path.clone(),
        &mut pending,
    );
    pending
        .get_mut(&source_path)
        .expect("source should be pending again")
        .stable_since = Instant::now() - STABILITY_DELAY;
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        (&in_flight, &undo_exemptions),
        &app_state_db,
        &shutdown,
    );

    assert!(event_receiver.try_recv().is_err());
    assert_eq!(
        app_state_db
            .list_organizer_operations(10)
            .expect("read operation history")
            .len(),
        1
    );
}

#[test]
fn missing_conflict_folder_keeps_the_source_pending_without_dispatch() {
    let root = tempfile::tempdir().expect("create test directory");
    let source_folder = root.path().join("source");
    let destination_folder = root.path().join("destination");
    let conflict_folder = root.path().join("missing-conflicts");
    std::fs::create_dir(&source_folder).expect("create source directory");
    std::fs::create_dir(&destination_folder).expect("create destination directory");
    let source_path = source_folder.join("report.txt");
    std::fs::write(&source_path, b"source").expect("create source file");
    let rules = vec![OrganizerRule::from_persisted_with_policy(
        1,
        source_folder,
        destination_folder,
        vec!["txt".to_string()],
        true,
        OrganizerConflictPolicy::MoveToConflictFolder(conflict_folder),
    )
    .expect("create persisted rule")];
    let activations = activation_flags_for(&rules);
    let paused_rules = HashSet::new();
    let mut pending = HashMap::new();
    queue_matching_path(
        &rules,
        &activations,
        &paused_rules,
        source_path.clone(),
        &mut pending,
    );
    pending
        .get_mut(&source_path)
        .expect("source should be pending")
        .stable_since = Instant::now() - STABILITY_DELAY;

    let (operation_sender, operation_receiver) = crossbeam_channel::unbounded();
    let (event_sender, event_receiver) = std::sync::mpsc::channel();
    let in_flight = OrganizerInFlightRegistry::default();
    let undo_exemptions = OrganizerUndoExemptionRegistry::default();
    let app_state_db = Arc::new(AppStateDb::new_in_memory().expect("database"));
    let shutdown = Arc::new(AtomicBool::new(false));
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        (&in_flight, &undo_exemptions),
        &app_state_db,
        &shutdown,
    );

    assert!(pending.contains_key(&source_path));
    assert!(operation_receiver.try_recv().is_err());
    assert!(event_receiver.try_recv().is_err());
    assert!(app_state_db
        .list_organizer_operations(10)
        .expect("read operation history")
        .is_empty());
}
