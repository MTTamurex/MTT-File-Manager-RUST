use crate::app::state::ImageViewerApp;
use std::time::Instant;

impl ImageViewerApp {
    pub fn trigger_manual_refresh(&mut self) {
        if self.navigation_state.is_computer_view {
            self.reload_drive_list_async();
            self.drive_state.last_drive_refresh = Instant::now();
        } else if self.navigation_state.is_recycle_bin_view {
            self.setup_recycle_bin_view();
        } else {
            // Force regeneration of visible folder previews and folder sizes on
            // manual refresh. This avoids reusing stale in-memory values for
            // folders whose contents changed while the app was offline or when
            // watcher invalidation was missed.
            let visible_folder_paths: Vec<_> = self
                .all_items
                .iter()
                .filter(|item| item.is_dir)
                .map(|item| item.path.clone())
                .collect();
            for folder_path in visible_folder_paths {
                self.invalidate_folder_size_cache_without_revalidation(&folder_path);
                self.cache_manager.invalidate_folder_preview(&folder_path);
                self.disk_cache.remove_folder_preview_cache(&folder_path);
            }

            // Clear loaded_path to force reload even if path hasn't changed
            self.loaded_path.clear();
            self.load_folder(true);
        }
    }
}
