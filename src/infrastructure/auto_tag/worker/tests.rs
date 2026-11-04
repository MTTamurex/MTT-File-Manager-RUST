use super::*;
use crate::domain::file_tag::TagColor;
use crate::infrastructure::app_state_db::AppStateDb;
use std::sync::mpsc::channel;

struct ScanFixture {
    _tempdir: tempfile::TempDir,
    context: AutoTagContext,
    events: Receiver<AutoTagEvent>,
    defs: FxHashMap<i64, FileTag>,
    rule: AutoTagRule,
    file: PathBuf,
    folder: PathBuf,
    nested_file: PathBuf,
}

fn scan_fixture(recursive: bool) -> ScanFixture {
    let tempdir = tempfile::tempdir().unwrap();
    let root = tempdir.path().to_path_buf();
    let file = root.join("file.txt");
    let folder = root.join("Subfolder");
    let nested_file = folder.join("nested.txt");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&file, b"x").unwrap();
    std::fs::write(&nested_file, b"x").unwrap();
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let tag_id = db.create_tag("Test tag", TagColor::Red, None).unwrap();
    let defs = db
        .get_all_tags()
        .into_iter()
        .map(|tag| (tag.id, tag))
        .collect();
    let (event_sender, events) = channel();
    ScanFixture {
        _tempdir: tempdir,
        context: AutoTagContext {
            app_state_db: db,
            event_sender,
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
        events,
        defs,
        rule: AutoTagRule {
            id: 1,
            folder_path: root,
            tag_id,
            enabled: true,
            include_subfolders: recursive,
            position: 0,
        },
        file,
        folder,
        nested_file,
    }
}

fn applied(fixture: &ScanFixture) -> FxHashSet<PathBuf> {
    let mut paths = FxHashSet::default();
    while let Ok(event) = fixture.events.try_recv() {
        if let AutoTagEvent::Applied { assignments } = event {
            paths.extend(assignments.into_iter().map(|(path, _)| path));
        }
    }
    paths
}

#[test]
fn startup_scan_without_subfolders_tags_files_only() {
    let f = scan_fixture(false);
    scan_and_apply(&f.context, std::slice::from_ref(&f.rule), &f.defs, None);
    assert_eq!(applied(&f), [f.file].into_iter().collect());
}

#[test]
fn startup_scan_with_subfolders_covers_the_tree_but_not_the_root() {
    let f = scan_fixture(true);
    scan_and_apply(&f.context, std::slice::from_ref(&f.rule), &f.defs, None);
    assert_eq!(
        applied(&f),
        [f.file, f.folder, f.nested_file].into_iter().collect()
    );
}

#[test]
fn arriving_directory_also_tags_its_existing_contents() {
    let f = scan_fixture(true);
    let mut pending = [(f.folder.clone(), Instant::now() - STABILITY_DELAY)]
        .into_iter()
        .collect();
    assert!(process_stable_files(
        &f.context,
        std::slice::from_ref(&f.rule),
        &f.defs,
        &mut pending
    ));
    assert_eq!(applied(&f), [f.folder, f.nested_file].into_iter().collect());
}

#[test]
fn shared_folder_uses_recursive_watch_regardless_of_rule_order() {
    let f = scan_fixture(false);
    let mut recursive = f.rule.clone();
    recursive.id = 2;
    recursive.include_subfolders = true;
    for rules in [
        vec![f.rule.clone(), recursive.clone()],
        vec![recursive, f.rule.clone()],
    ] {
        assert_eq!(
            watch_targets(&rules),
            vec![(f.rule.folder_path.clone(), true)]
        );
    }
}

#[test]
fn overlapping_scans_do_not_tag_rule_roots() {
    let f = scan_fixture(true);
    let mut other = f.rule.clone();
    other.id = 2;
    scan_and_apply(&f.context, &[f.rule.clone(), other], &f.defs, None);
    assert!(!applied(&f).contains(&f.rule.folder_path));
}

#[test]
fn cancelled_scan_does_not_apply_assignments() {
    let f = scan_fixture(true);
    f.context.shutdown.store(true, Ordering::Release);
    scan_and_apply(&f.context, std::slice::from_ref(&f.rule), &f.defs, None);
    assert!(applied(&f).is_empty());
    assert!(f.context.app_state_db.get_all_tag_assignments().is_empty());
}

#[test]
fn unknown_tags_do_not_produce_applied_events() {
    let f = scan_fixture(true);
    scan_and_apply(
        &f.context,
        std::slice::from_ref(&f.rule),
        &FxHashMap::default(),
        None,
    );
    let mut pending = [(f.file.clone(), Instant::now() - STABILITY_DELAY)]
        .into_iter()
        .collect();
    assert!(!process_stable_files(
        &f.context,
        std::slice::from_ref(&f.rule),
        &FxHashMap::default(),
        &mut pending
    ));
    assert!(applied(&f).is_empty());
}

#[test]
fn root_metadata_events_do_not_tag_the_watched_folder_itself() {
    let f = scan_fixture(true);
    let mut pending = [(f.rule.folder_path.clone(), Instant::now() - STABILITY_DELAY)]
        .into_iter()
        .collect();
    process_stable_files(
        &f.context,
        std::slice::from_ref(&f.rule),
        &f.defs,
        &mut pending,
    );
    let paths = applied(&f);
    assert!(!paths.contains(&f.rule.folder_path));
    assert!(paths.contains(&f.nested_file));
}

#[test]
fn single_path_rename_with_unknown_direction_is_evaluated() {
    let f = scan_fixture(true);
    let event = notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Name(
        notify::event::RenameMode::Any,
    )))
    .add_path(f.file.clone());
    let mut pending = FxHashMap::default();
    handle_notify_event(&event, &[f.rule], &mut pending);
    assert!(pending.contains_key(&f.file));
}

#[test]
fn native_watcher_tags_contents_of_a_directory_moved_into_the_rule_folder() {
    let f = scan_fixture(true);
    let mut non_recursive = f.rule.clone();
    non_recursive.id = 2;
    non_recursive.include_subfolders = false;
    let manager = crate::infrastructure::auto_tag::AutoTagManager::start(
        Arc::clone(&f.context.app_state_db),
        vec![non_recursive, f.rule.clone()],
        f.defs.values().cloned().collect(),
        eframe::egui::Context::default(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let event = manager
            .event_receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if matches!(
            event,
            AutoTagEvent::Status {
                status: AutoTagRuleStatus::Watching,
                ..
            }
        ) {
            break;
        }
    }
    let staging = tempfile::tempdir().unwrap();
    let source = staging.path().join("Incoming");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file.txt"), b"x").unwrap();
    let destination = f.rule.folder_path.join("Incoming");
    let target = destination.join("file.txt");
    std::fs::rename(source, destination).unwrap();
    loop {
        let event = manager
            .event_receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if let AutoTagEvent::Applied { assignments } = event {
            if assignments.iter().any(|(path, _)| path == &target) {
                break;
            }
        }
    }
    assert!(f
        .context
        .app_state_db
        .get_all_tag_assignments()
        .contains_key(&target));
}
