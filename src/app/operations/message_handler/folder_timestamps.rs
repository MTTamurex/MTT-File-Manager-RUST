//! Route background folder dates to matching live and stored browsing states.

use crate::app::{dual_panel::PanelSnapshot, state::ImageViewerApp};
use std::path::{Path, PathBuf};

fn apply_hints(
    current_path: &str,
    modified_hint: &mut Option<(PathBuf, u64)>,
    created_hint: &mut Option<(PathBuf, u64)>,
    resolved_path: &Path,
    modified: u64,
    created: Option<u64>,
) {
    if Path::new(current_path) != resolved_path {
        return;
    }
    if modified > 0 {
        *modified_hint = Some((resolved_path.to_path_buf(), modified));
    }
    if let Some(created) = created.filter(|value| *value > 0) {
        *created_hint = Some((resolved_path.to_path_buf(), created));
    }
}

fn apply_snapshot(snapshot: &mut PanelSnapshot, path: &Path, modified: u64, created: Option<u64>) {
    apply_hints(
        &snapshot.path,
        &mut snapshot.current_folder_modified_hint,
        &mut snapshot.current_folder_created_hint,
        path,
        modified,
        created,
    );
}

impl ImageViewerApp {
    pub(super) fn apply_resolved_folder_timestamps(
        &mut self,
        path: &Path,
        modified: u64,
        created: Option<u64>,
    ) {
        if modified > 0 {
            self.folder_modified_hints.put(path.to_path_buf(), modified);
        }
        if let Some(created) = created.filter(|value| *value > 0) {
            self.folder_created_hints.put(path.to_path_buf(), created);
        }
        apply_hints(
            &self.navigation_state.current_path,
            &mut self.current_folder_modified_hint,
            &mut self.current_folder_created_hint,
            path,
            modified,
            created,
        );
        if let Some(snapshot) = &mut self.dual_panel_inactive_state {
            apply_snapshot(snapshot, path, modified, created);
        }
        for tab in &mut self.tab_manager.tabs {
            apply_hints(
                &tab.path,
                &mut tab.current_folder_modified_hint,
                &mut tab.current_folder_created_hint,
                path,
                modified,
                created,
            );
            if let Some(snapshot) = &mut tab.dual_panel_inactive_state {
                apply_snapshot(snapshot, path, modified, created);
            }
            if let Some(panels) = &mut tab.dual_panel_tabs {
                for panel_tab in panels
                    .left
                    .tabs
                    .iter_mut()
                    .chain(panels.right.tabs.iter_mut())
                {
                    apply_snapshot(&mut panel_tab.snapshot, path, modified, created);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_folder_dates_fill_empty_state() {
        let path = Path::new(r"C:\Folder");
        let mut modified = None;
        let mut created = None;
        apply_hints(
            r"C:\Folder",
            &mut modified,
            &mut created,
            path,
            200,
            Some(100),
        );
        assert_eq!(modified, Some((path.to_path_buf(), 200)));
        assert_eq!(created, Some((path.to_path_buf(), 100)));
    }

    #[test]
    fn late_result_does_not_replace_another_folders_dates() {
        let current = Path::new(r"D:\Folder");
        let mut modified = Some((current.to_path_buf(), 300));
        let mut created = Some((current.to_path_buf(), 150));
        apply_hints(
            r"D:\Folder",
            &mut modified,
            &mut created,
            Path::new(r"C:\Folder"),
            200,
            Some(100),
        );
        assert_eq!(modified, Some((current.to_path_buf(), 300)));
        assert_eq!(created, Some((current.to_path_buf(), 150)));
    }

    #[test]
    fn partial_result_preserves_known_dates() {
        let path = Path::new(r"C:\Folder");
        let mut modified = Some((path.to_path_buf(), 200));
        let mut created = Some((path.to_path_buf(), 100));
        for missing in [None, Some(0)] {
            apply_hints(r"C:\Folder", &mut modified, &mut created, path, 0, missing);
            assert_eq!(modified, Some((path.to_path_buf(), 200)));
            assert_eq!(created, Some((path.to_path_buf(), 100)));
        }
    }
}
