//! Open/close operations for the disk usage analyzer view.

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// Open the analyzer full view for an NTFS drive and start fetching.
    pub(crate) fn open_disk_analysis(&mut self, drive_letter: char) {
        self.context_menu.close();
        self.disk_analysis.active = true;
        self.disk_analysis.request(drive_letter);
    }

    /// Close the analyzer view and release the model.
    pub(crate) fn close_disk_analysis(&mut self) {
        self.disk_analysis.close();
    }
}
