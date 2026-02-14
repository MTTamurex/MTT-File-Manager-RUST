use crate::app::state::ImageViewerApp;
use std::time::Instant;

impl ImageViewerApp {
    pub fn trigger_manual_refresh(&mut self) {
        if self.is_computer_view {
            self.reload_drive_list_async();
            self.last_drive_refresh = Instant::now();
        } else if self.is_recycle_bin_view {
            self.setup_recycle_bin_view();
        } else {
            // Force regeneration of visible folder previews on manual refresh.
            // This avoids reusing stale SQLite previews for folders whose content changed
            // while the app was offline or when watcher invalidation was missed.
            for folder_path in self
                .all_items
                .iter()
                .filter(|item| item.is_dir)
                .map(|item| item.path.clone())
            {
                self.cache_manager.invalidate_folder_preview(&folder_path);
                self.disk_cache.remove_folder_preview_cache(&folder_path);
            }

            // Clear loaded_path to force reload even if path hasn't changed
            self.loaded_path.clear();
            self.load_folder(true);
        }
    }
}
