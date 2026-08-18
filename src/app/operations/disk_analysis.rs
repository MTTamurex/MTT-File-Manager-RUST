//! Open operation for the disk usage analyzer view.

use crate::app::disk_analysis_state::AnalyzerDriveSummary;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// Open the analyzer window for an NTFS drive and start fetching.
    ///
    /// Snapshots the drive facts the analyzer sidebar/header need: the
    /// deferred viewport callback must not borrow the main app state.
    pub(crate) fn open_disk_analysis(&mut self, drive_letter: char) {
        self.context_menu.close();

        let mut drives = Vec::new();
        for (path, label) in self.drive_state.disks.clone() {
            let Some(letter) = path.chars().next().filter(|c| c.is_ascii_alphabetic()) else {
                continue;
            };
            let info = self.drive_state.cached_drive_info(&path);
            let (file_system, total_space, free_space) = info
                .as_ref()
                .map(|i| (i.file_system.clone(), i.total_space, i.free_space))
                .unwrap_or_default();
            drives.push(AnalyzerDriveSummary {
                letter: letter.to_ascii_uppercase(),
                label,
                file_system,
                total_space,
                free_space,
            });
        }

        let mut state = self.disk_analysis.lock();
        state.drives = drives;
        state.active = true;
        state.request(drive_letter);
    }
}
