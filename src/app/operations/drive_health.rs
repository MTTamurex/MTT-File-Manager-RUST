use std::path::Path;
use std::time::Instant;

use crate::app::drive_state::{
    apply_drive_health_snapshot, normalize_drive_root_key, DriveHealthResult,
};
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::DriveInfo;
use crate::infrastructure::windows::DriveType;

fn exact_drive_root(path: &Path) -> Option<String> {
    crate::infrastructure::windows::normalize_drive_root_path(path)
}

fn supports_drive_health(drive_type: DriveType) -> bool {
    matches!(drive_type, DriveType::Fixed | DriveType::Removable)
}

fn panel_drive_health_target(
    is_computer_view: bool,
    is_recycle_bin_view: bool,
    current_path: &str,
    selected_path: Option<&Path>,
) -> Option<String> {
    if let Some(selected_path) = selected_path {
        return is_computer_view
            .then(|| exact_drive_root(selected_path))
            .flatten();
    }
    if is_computer_view || is_recycle_bin_view {
        return None;
    }
    exact_drive_root(Path::new(current_path))
}

impl ImageViewerApp {
    pub fn request_drive_health_for_panel(&mut self) {
        if !self.show_preview_panel {
            return;
        }
        let Some(root) = panel_drive_health_target(
            self.navigation_state.is_computer_view,
            self.navigation_state.is_recycle_bin_view,
            &self.navigation_state.current_path,
            self.selected_file.as_ref().map(|file| file.path.as_path()),
        ) else {
            return;
        };

        let drive_type = self
            .selected_file
            .as_ref()
            .filter(|file| {
                normalize_drive_root_key(&file.path.to_string_lossy()).as_ref() == Some(&root)
            })
            .and_then(|file| file.drive_info.as_ref())
            .map(|info| info.drive_type)
            .or_else(|| {
                self.drive_state
                    .cached_drive_info(&root)
                    .map(|info| info.drive_type)
            })
            .unwrap_or_else(|| crate::infrastructure::windows::detect_drive_type(&root));
        if !supports_drive_health(drive_type) {
            return;
        }

        let now = Instant::now();
        if let Some(snapshot) = self.drive_state.cached_drive_health(&root, now) {
            let needs_hydration = self
                .selected_file
                .as_ref()
                .filter(|file| exact_drive_root(&file.path).as_ref() == Some(&root))
                .and_then(|file| file.drive_info.as_ref())
                .and_then(|info| info.health.as_ref())
                != Some(&snapshot);
            if needs_hydration {
                if let Some(mut info) = self.drive_state.cached_drive_info(&root) {
                    apply_drive_health_snapshot(&mut info, snapshot);
                    self.drive_state.cache_drive_info(&root, info.clone());
                    self.apply_drive_info_to_current_computer_items(&[(root, info)]);
                }
            }
            return;
        }

        let Some(request_id) = self.drive_state.begin_drive_health_request(&root, now) else {
            return;
        };
        let tx = self.drive_state.drive_health_tx.clone();
        let ctx = self.ui_ctx.clone();
        std::thread::spawn(move || {
            let result =
                if crate::infrastructure::io_priority::is_virtual_drive_path(Path::new(&root)) {
                    Err("Drive health is unavailable for virtual drives".to_string())
                } else {
                    let drive_letter = root
                        .chars()
                        .next()
                        .expect("normalized drive root has a letter");
                    crate::infrastructure::global_search::drive_health(drive_letter)
                };
            let _ = tx.send(DriveHealthResult {
                root,
                request_id,
                completed_at: Instant::now(),
                result,
            });
            ctx.request_repaint();
        });
    }

    pub fn poll_drive_health(&mut self) {
        let mut applied = Vec::new();
        while let Ok(message) = self.drive_state.drive_health_rx.try_recv() {
            if !self
                .drive_state
                .finish_drive_health_request(&message.root, message.request_id)
            {
                continue;
            }
            let Some(root) = self.drive_state.canonical_current_drive(&message.root) else {
                self.drive_state.invalidate_drive_health(&message.root);
                continue;
            };

            match message.result {
                Ok(snapshot)
                    if snapshot.drive_letter.to_ascii_uppercase()
                        == root.chars().next().unwrap_or_default() =>
                {
                    let mut info = self
                        .drive_state
                        .cached_drive_info(&root)
                        .unwrap_or(DriveInfo {
                            file_system: String::new(),
                            total_space: 0,
                            free_space: 0,
                            drive_type: crate::infrastructure::windows::detect_drive_type(&root),
                            model: None,
                            serial_number: None,
                            firmware_revision: None,
                            bus_type: None,
                            health: None,
                        });
                    apply_drive_health_snapshot(&mut info, snapshot.clone());
                    self.drive_state
                        .cache_drive_health(&root, snapshot, message.completed_at);
                    self.drive_state.cache_drive_info(&root, info.clone());
                    applied.push((root, info));
                }
                Ok(snapshot) => {
                    log::warn!(
                        "[DRIVE-HEALTH] Ignoring letter mismatch for {}: {}",
                        root,
                        snapshot.drive_letter
                    );
                    self.drive_state
                        .record_drive_health_failure(&root, Instant::now());
                    if let Some(info) = self.drive_state.remove_drive_health_snapshot(&root) {
                        applied.push((root, info));
                    }
                }
                Err(error) => {
                    log::debug!("[DRIVE-HEALTH] Query failed for {}: {}", root, error);
                    self.drive_state
                        .record_drive_health_failure(&root, Instant::now());
                    if let Some(info) = self.drive_state.remove_drive_health_snapshot(&root) {
                        applied.push((root, info));
                    }
                }
            }
        }

        if applied.is_empty() {
            return;
        }
        self.apply_drive_info_to_current_computer_items(&applied);
        if self.dual_panel_enabled
            && self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| snapshot.is_computer_view)
        {
            self.with_inactive_panel(|app| {
                app.apply_drive_info_to_current_computer_items(&applied);
            });
        }
        self.ui_ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_target_is_selected_computer_drive_or_unselected_current_root() {
        assert_eq!(
            panel_drive_health_target(true, false, "Este Computador", Some(Path::new("d:\\"))),
            Some("D:\\".to_string())
        );
        assert_eq!(
            panel_drive_health_target(false, false, "c:\\", None),
            Some("C:\\".to_string())
        );
        assert_eq!(
            panel_drive_health_target(false, false, "C:\\folder", None),
            None
        );
        assert_eq!(
            panel_drive_health_target(false, false, "C:\\", Some(Path::new("C:\\file.txt"))),
            None
        );
    }

    #[test]
    fn panel_target_rejects_virtual_views_and_non_drive_selection() {
        assert_eq!(
            panel_drive_health_target(true, false, "Este Computador", None),
            None
        );
        assert_eq!(
            panel_drive_health_target(true, false, "Este Computador", Some(Path::new("tag:item"))),
            None
        );
        assert_eq!(
            panel_drive_health_target(false, true, "Lixeira", None),
            None
        );
    }

    #[test]
    fn only_physical_drive_types_support_health_requests() {
        assert!(supports_drive_health(DriveType::Fixed));
        assert!(supports_drive_health(DriveType::Removable));
        assert!(!supports_drive_health(DriveType::Remote));
        assert!(!supports_drive_health(DriveType::Unknown));
        assert!(!supports_drive_health(DriveType::Cdrom));
        assert!(!supports_drive_health(DriveType::RamDisk));
    }
}
