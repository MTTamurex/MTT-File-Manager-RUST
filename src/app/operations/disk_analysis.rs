//! Spawns the standalone disk usage analyzer process.

use crate::app::state::ImageViewerApp;
use crate::application::disk_analysis_policy::is_local_volume;
use crate::infrastructure::windows::detect_drive_type;

impl ImageViewerApp {
    /// Open the analyzer as its own OS process (same model as the dedicated
    /// viewers: independent taskbar button and minimize/restore lifecycle).
    pub(crate) fn open_disk_analysis(&mut self, drive_letter: char) {
        self.context_menu.close();
        // Recheck the current drive type: a menu may outlive its drive mapping.
        if !drive_letter.is_ascii_alphabetic()
            || !is_local_volume(detect_drive_type(&format!("{drive_letter}:\\")))
        {
            return;
        }

        let exe = match std::env::current_exe() {
            Ok(v) => v,
            Err(err) => {
                log::error!("[DISK-ANALYZER] failed to locate current executable for spawn: {err}");
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
