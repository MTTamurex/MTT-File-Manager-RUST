use super::*;

#[test]
fn selection_preserves_subtree_minimum_empty_and_singleton_rules() {
    let (_dir, model, _) = fixture(&[Vec::new(), vec![1; 20], vec![2; 20], vec![3; 100]]);
    let (groups, skipped) = candidates::collect(&model, model.root, 10, || false).unwrap();
    assert_eq!(skipped, 1);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, 20);
    assert_eq!(groups[0].1.len(), 2);
    let (groups, skipped) = candidates::collect(&model, model.root, 21, || false).unwrap();
    assert!(groups.is_empty());
    assert_eq!(skipped, 3);
    let file = model
        .nodes
        .iter()
        .position(|n| n.name == "file-1.bin")
        .unwrap() as u32;
    assert!(candidates::collect(&model, file, 1, || false)
        .unwrap()
        .0
        .is_empty());
    assert!(candidates::collect(&model, model.root, 1, || true).is_none());
}

#[test]
fn reparse_files_and_children_below_reparse_directories_never_become_candidates() {
    let record = |frn, parent_frn, name: &str, is_dir, is_reparse| DiskAnalysisRecord {
        frn,
        parent_frn,
        name: name.into(),
        size: if is_dir { 0 } else { 100 },
        allocated_size: if is_dir { 0 } else { 100 },
        is_dir,
        is_reparse,
    };
    let model = DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'C',
        records: vec![
            record(5, 5, "", true, false),
            record(6, 5, "junction", true, true),
            record(7, 6, "inside.bin", false, false),
            record(8, 5, "link.bin", false, true),
            record(9, 5, "file.bin", false, false),
        ],
    });
    let (groups, _) = candidates::collect(&model, model.root, 1, || false).unwrap();
    assert!(
        groups.is_empty(),
        "Only one ordinary reachable file remains"
    );
}

#[test]
fn default_mode_never_guesses_ssd_capabilities_and_parallel_limits_are_separate() {
    let session = DuplicateSession::new();
    assert_eq!(session.read_mode, DuplicateReadMode::Conservative);
    let limits = session.read_mode.limits();
    assert_eq!((limits.partial, limits.full), (1, 1));
    let limits = DuplicateReadMode::Parallel.limits();
    assert_eq!((limits.partial, limits.full), (4, 2));
}
