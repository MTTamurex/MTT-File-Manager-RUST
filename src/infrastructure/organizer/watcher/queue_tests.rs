use super::*;
use notify::event::ModifyKind;
use notify::{Event, EventKind};

#[test]
fn matching_path_updates_rule_precedence_without_resetting_stability() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let first_destination = root.path().join("first");
    let second_destination = root.path().join("second");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&first_destination).expect("create first destination");
    std::fs::create_dir(&second_destination).expect("create second destination");
    let source_path = source.join("report.txt");
    std::fs::write(&source_path, b"source").expect("create source file");

    let first = OrganizerRule::from_persisted(
        1,
        source.clone(),
        first_destination,
        vec!["txt".to_string()],
        true,
    )
    .expect("create first rule");
    let second =
        OrganizerRule::from_persisted(2, source, second_destination, vec!["txt".to_string()], true)
            .expect("create second rule");
    let rules = vec![first.clone(), second.clone()];
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
    let stable_since = pending[&source_path].stable_since;
    queue_matching_path(
        &[second, first],
        &activations,
        &paused_rules,
        source_path.clone(),
        &mut pending,
    );

    assert_eq!(pending[&source_path].rule.id, 2);
    assert_eq!(pending[&source_path].stable_since, stable_since);
}

#[test]
fn in_flight_move_suppresses_duplicate_watcher_dispatch() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&destination).expect("create destination directory");
    let source_path = source.join("report.txt");
    std::fs::write(&source_path, b"source").expect("create source file");
    let rules =
        vec![
            OrganizerRule::from_persisted(1, source, destination, vec!["txt".to_string()], true)
                .expect("create rule"),
        ];
    let activations = activation_flags_for(&rules);
    let paused_rules = HashSet::new();
    let in_flight = OrganizerInFlightRegistry::default();
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
    let (event_sender, _event_receiver) = std::sync::mpsc::channel();
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        &in_flight,
    );
    let request = operation_receiver.try_recv().expect("move request");

    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(source_path.clone());
    process_watcher_event(&event, &rules, &activations, &paused_rules, &mut pending);
    assert!(pending.contains_key(&source_path));
    pending
        .get_mut(&source_path)
        .expect("source should remain deferred")
        .stable_since = Instant::now() - STABILITY_DELAY;
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        &in_flight,
    );
    assert!(operation_receiver.try_recv().is_err());
    assert!(pending.contains_key(&source_path));

    drop(request);
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        &in_flight,
    );
    assert!(operation_receiver.try_recv().is_ok());
    assert!(pending.is_empty());
}

#[test]
fn unavailable_destination_does_not_dispatch() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("missing-destination");
    std::fs::create_dir(&source).expect("create source directory");
    let source_path = source.join("report.txt");
    std::fs::write(&source_path, b"source").expect("create source file");
    let rules =
        vec![
            OrganizerRule::from_persisted(1, source, destination, vec!["txt".to_string()], true)
                .expect("create rule"),
        ];
    let activations = activation_flags_for(&rules);
    let paused_rules = HashSet::new();
    let in_flight = OrganizerInFlightRegistry::default();
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
        &in_flight,
    );

    assert!(operation_receiver.try_recv().is_err());
    assert!(event_receiver.try_recv().is_err());
    assert!(pending.contains_key(&source_path));

    std::fs::create_dir(&rules[0].destination_folder).expect("restore destination");
    pending
        .get_mut(&source_path)
        .expect("source should remain pending")
        .stable_since = Instant::now() - STABILITY_DELAY;
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        &in_flight,
    );
    assert!(operation_receiver.try_recv().is_ok());
}
