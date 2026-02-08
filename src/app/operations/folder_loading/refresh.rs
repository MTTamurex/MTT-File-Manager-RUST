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
            // Clear loaded_path to force reload even if path hasn't changed
            self.loaded_path.clear();
            self.load_folder(true);
        }
    }
}
