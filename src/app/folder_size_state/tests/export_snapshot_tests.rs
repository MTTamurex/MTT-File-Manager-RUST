use super::*;

#[test]
fn export_snapshot_excludes_partial_values_but_keeps_completed_sort_sizes() {
    let mut state = test_state();
    let partial = PathBuf::from("folder-partial");
    let complete = PathBuf::from("folder-complete");
    let sort = PathBuf::from("folder-sorted");
    state.batch_cache.put(partial.clone(), 40);
    state
        .cache
        .put(partial.clone(), FolderContentSummary::size_only(40));
    state.loading.insert(partial.clone());
    state.batch_progress.insert(partial.clone(), 40);
    state.cache_completed_folder_size(complete.clone(), 100);
    state.size_sorting.sizes.insert(sort.clone(), 200);
    state.loading.insert(sort.clone());
    state.batch_cache.put(sort.clone(), 20);
    let snapshot = state.export_size_snapshot();
    assert!(!snapshot.contains_key(&crate::domain::file_tag::normalize_tag_path_key(&partial)));
    assert_eq!(
        snapshot.get(&crate::domain::file_tag::normalize_tag_path_key(&complete)),
        Some(&100)
    );
    assert_eq!(
        snapshot.get(&crate::domain::file_tag::normalize_tag_path_key(&sort)),
        Some(&200)
    );
}

#[test]
fn export_snapshot_excludes_progress_with_different_path_casing() {
    let mut state = test_state();
    state.batch_cache.put(PathBuf::from(r"C:\Folder"), 40);
    state.loading.insert(PathBuf::from(r"c:\folder"));
    assert!(state.export_size_snapshot().is_empty());
}

#[test]
fn export_snapshot_preserves_completed_value_priority_across_path_aliases() {
    let mut state = test_state();
    state
        .size_sorting
        .sizes
        .insert(PathBuf::from(r"C:\Folder"), 200);
    state.batch_cache.put(PathBuf::from(r"\\?\c:\folder"), 40);
    let snapshot = state.export_size_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot.get(r"c:\folder"), Some(&200));
}

#[test]
fn export_snapshot_excludes_invalidated_aliases() {
    let mut state = test_state();
    state.batch_cache.put(PathBuf::from(r"C:\Folder"), 100);
    state.schedule_revalidation(PathBuf::from(r"c:\folder"), Instant::now());
    assert!(state.export_size_snapshot().is_empty());
}

#[test]
fn export_snapshot_does_not_treat_expired_failure_as_active() {
    let mut state = test_state();
    let path = PathBuf::from("folder");
    state.batch_cache.put(path.clone(), 100);
    state
        .failed_until
        .insert(path, Instant::now() - Duration::from_secs(1));
    assert_eq!(state.export_size_snapshot().get("folder"), Some(&100));
}
