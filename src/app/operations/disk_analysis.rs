//! Spawns the standalone disk usage analyzer process.

use crate::app::state::ImageViewerApp;
use crate::application::disk_analysis_policy::{is_local_volume, supports_disk_analysis};
use crate::infrastructure::windows::{detect_drive_type, get_file_system_for_path};
use std::path::Path;

impl ImageViewerApp {
    /// Open the analyzer as its own OS process (same model as the dedicated
    /// viewers: independent taskbar button and minimize/restore lifecycle).
    pub(crate) fn open_disk_analysis(&mut self, drive_letter: char) {
        self.context_menu.close();
        if !drive_letter.is_ascii_alphabetic() {
            return;
        }
        let drive_letter = drive_letter.to_ascii_uppercase();
        let root = format!("{drive_letter}:\\");
        // A menu or palette may outlive its drive mapping. Reject remote/unknown
        // drives before querying their filesystem, then recheck NTFS before spawning.
        let drive_type = detect_drive_type(&root);
        if !is_local_volume(drive_type) {
            return;
        }
        let file_system = get_file_system_for_path(Path::new(&root)).unwrap_or_default();
        if !supports_disk_analysis(Path::new(&root), drive_type, &file_system) {
            return;
        }

        let exe = match std::env::current_exe() {
            Ok(v) => v,
            Err(err) => {
                log::error!("[DISK-ANALYZER] failed to locate current executable for spawn: {err}");
                return;
            }
        };

        match crate::viewer_runtime::backend::viewer_command(exe).and_then(|mut command| {
            command
                .arg("--disk-analyzer")
                .arg(drive_letter.to_string())
                .spawn()
        }) {
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
