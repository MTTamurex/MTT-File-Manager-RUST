//! Spawns the standalone disk usage analyzer process.

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// Open the analyzer as its own OS process (same model as the dedicated
    /// viewers: independent taskbar button and minimize/restore lifecycle).
    pub(crate) fn open_disk_analysis(&mut self, drive_letter: char) {
        self.context_menu.close();

        let exe = match std::env::current_exe() {
            Ok(v) => v,
            Err(err) => {
                log::error!(
                    "[DISK-ANALYZER] failed to locate current executable for spawn: {err}"
                );
                return;
            }
        };

        match std::process::Command::new(exe)
            .arg("--disk-analyzer")
            .arg(drive_letter.to_string())
            .spawn()
        {
            Ok(child) => {
                let child_pid = child.id();
                crate::viewer_processes::register(child);
                log::info!(
                    "[DISK-ANALYZER] spawned standalone analyzer parent_pid={} child_pid={} drive={drive_letter}:",
                    std::process::id(),
                    child_pid
                );
            }
            Err(err) => {
                log::error!("[DISK-ANALYZER] failed to spawn analyzer process: {err}");
            }
        }
    }
}
