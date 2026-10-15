use crate::app::state::ImageViewerApp;
use std::path::Path;

impl ImageViewerApp {
    pub(crate) fn tab_path_staleness(&self, path: &Path) -> (bool, bool, bool) {
        let is_dirty = self.directory_dirty_registry.is_dirty(path);
        if is_dirty {
            return (true, true, false);
        }
        if crate::infrastructure::onedrive::is_cloud_sync_path(path) {
            let path_buf = path.to_path_buf();
            let is_stale = self
                .directory_cache
                .cached_at_ms(&path_buf)
                .map(|cached_at_ms| {
                    !crate::infrastructure::onedrive::directory_cache_is_recent(cached_at_ms)
                })
                .unwrap_or(true);
            return (false, is_stale, false);
        }
        if self.global_search.available {
            let path_string = path.to_string_lossy().into_owned();
            return match crate::infrastructure::global_search::check_paths_modified(
                &[path_string],
                120,
            ) {
                Ok(modified) => (false, !modified.is_empty(), true),
                Err(error) => {
                    log::debug!("[TAB] Search service check_paths_modified failed; skipping UI-thread mtime fallback: {}", error);
                    (false, false, false)
                }
            };
        }
        // Watcher and consistency probes handle uncached directories without
        // filesystem metadata calls on the UI thread.
        (false, false, false)
    }
}
