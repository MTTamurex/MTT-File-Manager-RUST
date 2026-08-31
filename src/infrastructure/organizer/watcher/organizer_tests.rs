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
    let mut pending = HashMap::new();
    queue_matching_path(&rules, &activations, source_path.clone(), &mut pending);
    pending
        .get_mut(&source_path)
        .expect("source should be pending")
        .stable_since = Instant::now() - STABILITY_DELAY;

    let (operation_sender, operation_receiver) = crossbeam_channel::unbounded();
    let (event_sender, event_receiver) = std::sync::mpsc::channel();
    process_stable_files(&mut pending, &operation_sender, &event_sender);
    assert!(pending.is_empty());
    assert!(matches!(
        event_receiver.try_recv(),
        Ok(OrganizerEvent::SkippedConflict { path }) if path == source_path
    ));

    let event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(destination_path.clone());
    process_watcher_event(&event, &rules, &activations, &mut pending);
    assert!(pending.is_empty());

    std::fs::rename(&destination_path, &renamed_destination_path).expect("rename destination file");
    let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::From)))
        .add_path(destination_path);
    process_watcher_event(&event, &rules, &activations, &mut pending);

    assert!(pending.contains_key(&source_path));

    pending
        .get_mut(&source_path)
        .expect("source should be requeued")
        .stable_since = Instant::now() - STABILITY_DELAY;
    process_stable_files(&mut pending, &operation_sender, &event_sender);
    match operation_receiver.try_recv() {
        Ok(FileOperationRequest::OrganizerMove {
            path, dest_folder, ..
        }) => {
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
    let mut pending = HashMap::new();

    std::fs::remove_file(&destination_path).expect("remove destination file");
    let event = Event::new(EventKind::Remove(RemoveKind::File)).add_path(destination_path);
    process_watcher_event(&event, &rules, &activations, &mut pending);

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
