use super::CacheManager;
use crate::workers::folder_preview_worker::CachedFolderPreview;
use std::path::PathBuf;

impl CacheManager {
    pub fn folder_preview_requires_validation(&self, path: &PathBuf) -> bool {
        self.folder_preview_refresh_required.contains(path)
    }

    pub fn mark_folder_preview_validated(&mut self, path: &PathBuf) {
        self.folder_preview_refresh_required.pop(path);
    }

    pub fn require_folder_preview_validation(&mut self, path: &PathBuf) {
        self.folder_preview_refresh_required.put(path.clone(), ());
        self.folder_preview_requests.invalidate(path);
    }

    pub fn get_folder_preview_rgba(&mut self, path: &PathBuf) -> Option<&CachedFolderPreview> {
        self.folder_preview_rgba_cache.get(path)
    }

    pub fn put_folder_preview_rgba(&mut self, path: PathBuf, preview: CachedFolderPreview) {
        let new_bytes = preview.pixels.len();
        self.remove_folder_preview_rgba(&path);
        if let Some((_, evicted)) = self.folder_preview_rgba_cache.push(path, preview) {
            self.folder_preview_rgba_bytes = self
                .folder_preview_rgba_bytes
                .saturating_sub(evicted.pixels.len());
        }
        self.folder_preview_rgba_bytes = self.folder_preview_rgba_bytes.saturating_add(new_bytes);
        // File thumbnails and folder previews share the dynamic RAM budget.
        self.enforce_folder_preview_rgba_budget(
            self.max_rgba_data_bytes
                .saturating_sub(self.rgba_data_bytes),
        );
    }

    pub fn remove_folder_preview_rgba(&mut self, path: &PathBuf) {
        if let Some(preview) = self.folder_preview_rgba_cache.pop(path) {
            self.folder_preview_rgba_bytes = self
                .folder_preview_rgba_bytes
                .saturating_sub(preview.pixels.len());
        }
    }

    /// Returns the number of evicted entries for memory-maintenance accounting.
    pub(super) fn enforce_folder_preview_rgba_budget(&mut self, budget_bytes: usize) -> usize {
        let mut removed = 0;
        let budget_bytes = budget_bytes.min(self.max_folder_preview_rgba_bytes);
        while self.folder_preview_rgba_bytes > budget_bytes {
            match self.folder_preview_rgba_cache.pop_lru() {
                Some((_, preview)) => {
                    self.folder_preview_rgba_bytes = self
                        .folder_preview_rgba_bytes
                        .saturating_sub(preview.pixels.len());
                    removed += 1;
                }
                None => {
                    self.folder_preview_rgba_bytes = 0;
                    break;
                }
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::folder_appearance::FolderAppearance;
    use crate::infrastructure::disk_cache::FolderPreviewSource;
    use std::path::Path;
    use std::sync::Arc;

    fn preview(folder: &Path, bytes: usize) -> CachedFolderPreview {
        CachedFolderPreview {
            pixels: Arc::new(vec![255; bytes]),
            width: 16,
            height: 16,
            appearance: FolderAppearance::Original,
            source: FolderPreviewSource::capture(folder, None).unwrap(),
        }
    }

    #[test]
    fn folder_preview_refresh_cancels_old_validation_without_dropping_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = CacheManager::new();
        let path = PathBuf::from("folder");
        cache.put_folder_preview_rgba(path.clone(), preview(dir.path(), 64));
        let old = cache.folder_preview_requests.begin(&path);
        cache.require_folder_preview_validation(&path);
        assert!(!old.is_current());
        assert!(cache.folder_preview_requires_validation(&path));
        assert!(cache.get_folder_preview_rgba(&path).is_some());
        assert!(cache.folder_preview_requests.begin(&path).is_current());
    }

    #[test]
    fn folder_preview_rgba_accounting_tracks_replacement_eviction_and_invalidation() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = CacheManager::new();
        cache
            .folder_preview_rgba_cache
            .resize(std::num::NonZeroUsize::new(1).unwrap());
        let first = PathBuf::from("folder-a");
        let second = PathBuf::from("folder-b");
        cache.put_folder_preview_rgba(first.clone(), preview(dir.path(), 64));
        assert_eq!(cache.estimate_ram_cache_usage(), 64);
        cache.put_folder_preview_rgba(first.clone(), preview(dir.path(), 32));
        assert_eq!(cache.estimate_ram_cache_usage(), 32);
        cache.put_folder_preview_rgba(second.clone(), preview(dir.path(), 16));
        assert_eq!(cache.estimate_ram_cache_usage(), 16);
        assert!(cache.get_folder_preview_rgba(&first).is_none());
        cache.note_folder_preview_request_sent(&second);
        cache.invalidate_folder_preview(&second);
        assert!(cache.folder_preview_requires_validation(&second));
        assert!(cache.get_folder_preview_rgba(&second).is_none());
        assert!(!cache.should_throttle_folder_preview_request(&second));
        assert_eq!(cache.estimate_ram_cache_usage(), 0);
        cache.mark_folder_preview_validated(&second);
        assert!(!cache.folder_preview_requires_validation(&second));
    }

    #[test]
    fn folder_preview_rgba_shares_budget_with_file_thumbnails() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = CacheManager::new();
        cache.max_rgba_data_bytes = 128;
        cache.put_folder_preview_rgba(PathBuf::from("folder-a"), preview(dir.path(), 80));
        cache.put_rgba_data(PathBuf::from("file.jpg"), Arc::new(vec![255; 80]), 4, 5);
        assert!(cache.estimate_ram_cache_usage() <= 128);
        assert!(cache.folder_preview_rgba_cache.is_empty());
        cache.put_folder_preview_rgba(PathBuf::from("folder-b"), preview(dir.path(), 80));
        assert!(cache.estimate_ram_cache_usage() <= 128);
    }

    #[test]
    fn folder_preview_rgba_is_trimmed_under_memory_pressure() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = CacheManager::new();
        cache.put_folder_preview_rgba(PathBuf::from("folder"), preview(dir.path(), 1024));
        let (_, removed, _) = cache.trim_thumbnail_caches(0, 0, 0, None);
        assert_eq!(removed, 1);
        assert_eq!(cache.estimate_ram_cache_usage(), 0);
        assert!(cache.folder_preview_rgba_cache.is_empty());
    }

    #[test]
    fn folder_preview_rgba_respects_dynamic_budget_reduction() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = CacheManager::new();
        cache.put_folder_preview_rgba(
            PathBuf::from("folder"),
            preview(dir.path(), super::super::MIN_RGBA_BUDGET_BYTES * 2),
        );
        let budget = cache.retune_rgba_budget(super::super::MIN_RGBA_BUDGET_BYTES);
        assert!(cache.estimate_ram_cache_usage() <= budget);
        assert!(cache.folder_preview_rgba_cache.is_empty());
    }

    #[test]
    fn folder_preview_rgba_release_reports_all_removed_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = CacheManager::new();
        cache.put_folder_preview_rgba(PathBuf::from("folder"), preview(dir.path(), 64));
        let (_, removed, _, bytes) = cache.release_thumbnail_caches_for_idle(1, 1, 1, 0);
        assert_eq!((removed, bytes), (1, 64));
        assert_eq!(cache.estimate_ram_cache_usage(), 0);
        cache.max_rgba_data_bytes = 128;
        cache.put_folder_preview_rgba(PathBuf::from("folder"), preview(dir.path(), 64));
        let (_, removed, _, bytes) =
            cache.release_thumbnail_cache_storage_for_background(1, 1, 1, 0);
        assert_eq!((removed, bytes), (1, 64));
        assert_eq!(cache.estimate_ram_cache_usage(), 0);
    }
}
