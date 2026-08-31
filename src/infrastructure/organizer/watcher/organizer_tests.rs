use super::*;
use notify::event::{ModifyKind, RemoveKind, RenameMode};
use notify::{Event, EventKind};
use std::path::PathBuf;

fn rule(id: i64, enabled: bool) -> OrganizerRule {
    OrganizerRule {
        id,
        source_folder: PathBuf::from(r"C:\Source"),
        destination_folder: PathBuf::from(r"C:\Destination"),
        extensions: vec!["txt".to_string()],
        enabled,
    }
}

#[test]
fn enabling_a_rule_marks_it_for_a_full_scan() {
    let previous = vec![rule(1, false)];
    let current = vec![rule(1, true)];
    let mut activations = activation_flags_for(&previous);

    let scan_rules = update_activation_flags(&previous, &current, &mut activations);

    assert_eq!(scan_rules, vec![1]);
    assert!(activations[&1].load(Ordering::Acquire));
}

#[test]
fn disabling_a_rule_deactivates_its_pending_work() {
    let previous = vec![rule(1, true)];
    let current = vec![rule(1, false)];
    let mut activations = activation_flags_for(&previous);

    let scan_rules = update_activation_flags(&previous, &current, &mut activations);

    assert!(scan_rules.is_empty());
    assert!(!activations[&1].load(Ordering::Acquire));
}

#[test]
fn changing_a_rule_invalidates_its_previous_activation() {
    let previous = vec![rule(1, true)];
    let mut current = previous.clone();
    current[0].destination_folder = PathBuf::from(r"C:\NewDestination");
    let mut activations = activation_flags_for(&previous);
    let previous_activation = activations[&1].clone();

    let scan_rules = update_activation_flags(&previous, &current, &mut activations);

    assert_eq!(scan_rules, vec![1]);
    assert!(!previous_activation.load(Ordering::Acquire));
    assert!(activations[&1].load(Ordering::Acquire));
}

#[test]
fn disabling_a_preferred_rule_rescans_enabled_fallback_rules() {
    let previous = vec![rule(1, true), rule(2, true)];
    let current = vec![rule(1, false), rule(2, true)];
    let mut activations = activation_flags_for(&previous);

    let scan_rules = update_activation_flags(&previous, &current, &mut activations);

    assert_eq!(scan_rules, vec![2]);
    assert!(!activations[&1].load(Ordering::Acquire));
    assert!(activations[&2].load(Ordering::Acquire));
}

#[test]
fn reordering_rules_cancels_all_previous_dispatch_tokens() {
    let previous = vec![rule(1, true), rule(2, true)];
    let current = vec![rule(2, true), rule(1, true)];
    let mut activations = activation_flags_for(&previous);
    let first_previous = activations[&1].clone();
    let second_previous = activations[&2].clone();

    let scan_rules = update_activation_flags(&previous, &current, &mut activations);

    assert_eq!(scan_rules, vec![2, 1]);
    assert!(!first_previous.load(Ordering::Acquire));
    assert!(!second_previous.load(Ordering::Acquire));
    assert!(activations[&1].load(Ordering::Acquire));
    assert!(activations[&2].load(Ordering::Acquire));
}

#[test]
fn destination_rename_requeues_a_conflicted_source() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&destination).expect("create destination directory");

    let source_path = source.join("report.txt");
    let destination_path = destination.join("report.txt");
    let renamed_destination_path = destination.join("old-report.txt");
    std::fs::write(&source_path, b"source").expect("create source file");
    std::fs::write(&destination_path, b"destination").expect("create destination file");

    let rule = OrganizerRule::from_persisted(
        1,
        source.clone(),
        destination.clone(),
        vec!["txt".to_string()],
        true,
    )
    .expect("create organizer rule");
    let rules = vec![rule];
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
    assert!(pending.is_empty());
    assert!(matches!(
        event_receiver.try_recv(),
        Ok(OrganizerEvent::OperationSkipped {
            operation_id,
            rule_id: 1,
            path,
        }) if operation_id.get() != 0 && path == source_path
    ));

    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(destination_path.clone());
    process_watcher_event(&event, &rules, &activations, &paused_rules, &mut pending);
    assert!(pending.is_empty());

    std::fs::rename(&destination_path, &renamed_destination_path).expect("rename destination file");
    let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
        .add_path(destination_path);
    process_watcher_event(&event, &rules, &activations, &paused_rules, &mut pending);

    assert!(pending.contains_key(&source_path));

    pending
        .get_mut(&source_path)
        .expect("source should be requeued")
        .stable_since = Instant::now() - STABILITY_DELAY;
    process_stable_files(
        &mut pending,
        &paused_rules,
        &operation_sender,
        &event_sender,
        &in_flight,
    );
    match operation_receiver.try_recv() {
        Ok(FileOperationRequest::OrganizerMove {
            operation_id,
            path,
            dest_folder,
            ..
        }) => {
            assert_ne!(operation_id.get(), 0);
            assert_eq!(path, source_path);
            assert_eq!(dest_folder, destination);
        }
        _ => panic!("expected organizer move request"),
    }
}

#[test]
fn destination_removal_requeues_source_without_prior_conflict_state() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&destination).expect("create destination directory");

    let source_path = source.join("report.txt");
    let destination_path = destination.join("report.txt");
    std::fs::write(&source_path, b"source").expect("create source file");
    std::fs::write(&destination_path, b"destination").expect("create destination file");
    let rule = OrganizerRule::from_persisted(1, source, destination, vec!["txt".to_string()], true)
        .expect("create organizer rule");
    let rules = vec![rule];
    let activations = activation_flags_for(&rules);
    let paused_rules = HashSet::new();
    let mut pending = HashMap::new();

    std::fs::remove_file(&destination_path).expect("remove destination file");
    let event = Event::new(EventKind::Remove(RemoveKind::File)).add_path(destination_path);
    process_watcher_event(&event, &rules, &activations, &paused_rules, &mut pending);

    assert!(pending.contains_key(&source_path));
}

#[test]
fn watched_folders_include_enabled_sources_and_destinations_once() {
    let first = rule(1, true);
    let mut second = rule(2, true);
    second.source_folder = PathBuf::from(r"c:/destination/");
    second.destination_folder = PathBuf::from(r"C:\Archive");
    let disabled = rule(3, false);

    assert_eq!(
        watched_folders(&[first, second, disabled]),
        vec![
            PathBuf::from(r"C:\Source"),
            PathBuf::from(r"C:\Destination"),
            PathBuf::from(r"C:\Archive"),
        ]
    );
}

#[test]
fn watched_path_normalization_handles_verbatim_unc_paths() {
    assert!(path_is_equal(
        std::path::Path::new(r"\\?\UNC\Server\Share\Destination"),
        std::path::Path::new(r"\\server\share\destination\"),
    ));
    assert!(path_is_equal(
        std::path::Path::new(r"\\?\C:\Destination"),
        std::path::Path::new(r"c:/destination/"),
    ));
}

#[test]
fn rule_status_distinguishes_missing_folders_and_pause_states() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("create source directory");
    let rule = OrganizerRule::from_persisted(1, source, destination, vec!["txt".to_string()], true)
        .expect("create organizer rule");
    let registered_folders = HashSet::from([
        normalize_watched_path(&rule.source_folder),
        normalize_watched_path(&rule.destination_folder),
    ]);

    assert_eq!(
        status_for_rule(&rule, &HashSet::new(), &registered_folders),
        OrganizerRuleStatus::DestinationUnavailable
    );
    assert_eq!(
        status_for_rule(&rule, &HashSet::from([rule.id]), &registered_folders),
        OrganizerRuleStatus::Paused
    );

    let mut disabled = rule;
    disabled.enabled = false;
    assert_eq!(
        status_for_rule(&disabled, &HashSet::new(), &registered_folders),
        OrganizerRuleStatus::Disabled
    );
}

#[test]
fn folder_creation_rejects_relative_and_namespace_paths() {
    assert!(is_safe_folder_creation_path(std::path::Path::new(
        r"C:\Folder"
    )));
    assert!(!is_safe_folder_creation_path(std::path::Path::new(
        "folder"
    )));
    assert!(!is_safe_folder_creation_path(std::path::Path::new(
        r"\\?\C:\Folder"
    )));
    assert!(!is_safe_folder_creation_path(std::path::Path::new(
        r"C:\Folder\..\Other"
    )));
}

#[test]
fn paused_rule_does_not_dispatch_pending_work() {
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
                .expect("create organizer rule"),
        ];
    let activations = activation_flags_for(&rules);
    let paused_rules = HashSet::from([1]);
    let mut pending = HashMap::new();
    queue_matching_path(
        &rules,
        &activations,
        &paused_rules,
        source_path,
        &mut pending,
    );
    assert!(pending.is_empty());
}

#[test]
fn active_status_requires_both_rule_folders_to_be_watched() {
    let root = tempfile::tempdir().expect("create test directory");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).expect("create source directory");
    std::fs::create_dir(&destination).expect("create destination directory");
    let rule = OrganizerRule::from_persisted(
        1,
        source.clone(),
        destination.clone(),
        vec!["txt".to_string()],
        true,
    )
    .expect("create organizer rule");
    let only_source = HashSet::from([normalize_watched_path(&source)]);
    let both = HashSet::from([
        normalize_watched_path(&source),
        normalize_watched_path(&destination),
    ]);

    assert_eq!(
        status_for_rule(&rule, &HashSet::new(), &only_source),
        OrganizerRuleStatus::Recovering
    );
    assert_eq!(
        status_for_rule(&rule, &HashSet::new(), &both),
        OrganizerRuleStatus::Active
    );
}
