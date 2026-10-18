use super::*;

#[test]
fn folder_preview_old_result_cannot_finish_new_loading_after_invalidation() {
    let mut cache = CacheManager::new();
    let path = PathBuf::from("folder");
    let old = cache.folder_preview_requests.begin(&path);
    cache.start_folder_preview_loading(path.clone());
    cache.invalidate_folder_preview(&path);
    let new = cache.folder_preview_requests.begin(&path);
    cache.start_folder_preview_loading(path.clone());
    // The result consumer discards invalid tokens before clearing loading state.
    assert!(!old.is_current());
    assert!(new.is_current());
    assert!(cache.is_folder_preview_loading(&path));
}

#[test]
fn folder_preview_stale_loading_retries_cancel_the_previous_job() {
    let mut cache = CacheManager::new();
    let path = PathBuf::from("folder");
    let old = cache.folder_preview_requests.begin(&path);
    cache.start_folder_preview_loading(path.clone());
    cache.folder_preview_loading_started.put(
        path.clone(),
        Instant::now() - FOLDER_PREVIEW_LOADING_STALE_AFTER,
    );
    assert_eq!(cache.prune_stale_folder_preview_loading(), 1);
    assert!(!old.is_current());
    assert!(cache.folder_preview_requests.begin(&path).is_current());
}
