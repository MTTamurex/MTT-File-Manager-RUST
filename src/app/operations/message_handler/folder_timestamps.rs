//! Route background folder dates to matching live and stored browsing states.

use crate::app::{dual_panel::PanelSnapshot, state::ImageViewerApp};
use crate::domain::file_tag::normalize_tag_path_key;
use std::path::{Path, PathBuf};

type Hint = Option<(PathBuf, u64)>;

fn visit_stored(
    inactive: &mut Option<PanelSnapshot>,
    tabs: &mut crate::tabs::TabManager,
    mut visit: impl FnMut(&str, &mut Hint, &mut Hint),
) {
    if let Some(snapshot) = inactive {
        visit(
            &snapshot.path,
            &mut snapshot.current_folder_modified_hint,
            &mut snapshot.current_folder_created_hint,
        );
    }
    for tab in &mut tabs.tabs {
        visit(
            &tab.path,
            &mut tab.current_folder_modified_hint,
            &mut tab.current_folder_created_hint,
        );
        if let Some(snapshot) = &mut tab.dual_panel_inactive_state {
            visit(
                &snapshot.path,
                &mut snapshot.current_folder_modified_hint,
                &mut snapshot.current_folder_created_hint,
            );
        }
        if let Some(panels) = &mut tab.dual_panel_tabs {
            for tab in panels
                .left
                .tabs
                .iter_mut()
                .chain(panels.right.tabs.iter_mut())
            {
                let snapshot = &mut tab.snapshot;
                visit(
                    &snapshot.path,
                    &mut snapshot.current_folder_modified_hint,
                    &mut snapshot.current_folder_created_hint,
                );
            }
        }
    }
}

fn apply_hints(
    current_path: &str,
    modified_hint: &mut Option<(PathBuf, u64)>,
    created_hint: &mut Option<(PathBuf, u64)>,
    resolved_path: &Path,
    modified: u64,
    created: Option<u64>,
) -> bool {
    if normalize_tag_path_key(Path::new(current_path)) != normalize_tag_path_key(resolved_path) {
        return false;
    }
    // Keep the owner's spelling: the preview cache compares its hint path to
    // current_path, even when the watcher used another case or extended prefix.
    let owner = PathBuf::from(current_path);
    if modified > 0 {
        *modified_hint = Some((owner.clone(), modified));
    }
    if let Some(created) = created.filter(|value| *value > 0) {
        *created_hint = Some((owner, created));
    }
    true
}

impl ImageViewerApp {
    pub(in crate::app::operations) fn visit_stored_folder_timestamps(
        &mut self,
        visit: impl FnMut(&str, &mut Hint, &mut Hint),
    ) {
        visit_stored(
            &mut self.dual_panel_inactive_state,
            &mut self.tab_manager,
            visit,
        );
    }

    pub(super) fn poll_folder_timestamps(&mut self, ctx: &eframe::egui::Context) {
        for (path, dates) in self.folder_timestamps.poll(ctx, std::time::Instant::now()) {
            self.apply_resolved_folder_timestamps(&path, dates.modified, dates.created);
            ctx.request_repaint();
        }
    }

    pub(super) fn apply_resolved_folder_timestamps(
        &mut self,
        path: &Path,
        modified: u64,
        created: Option<u64>,
    ) {
        let key = normalize_tag_path_key(path);
        let mut aliases: std::collections::HashSet<PathBuf> = self
            .folder_modified_hints
            .iter()
            .chain(self.folder_created_hints.iter())
            .filter(|(p, _)| normalize_tag_path_key(p) == key)
            .map(|(p, _)| p.clone())
            .collect();
        aliases.insert(path.to_path_buf());
        apply_hints(
            &self.navigation_state.current_path,
            &mut self.current_folder_modified_hint,
            &mut self.current_folder_created_hint,
            path,
            modified,
            created,
        );
        aliases.insert(PathBuf::from(&self.navigation_state.current_path));
        self.visit_stored_folder_timestamps(|owner, modified_hint, created_hint| {
            if apply_hints(owner, modified_hint, created_hint, path, modified, created) {
                aliases.insert(PathBuf::from(owner));
            }
        });
        for alias in aliases {
            if normalize_tag_path_key(&alias) != key {
                continue;
            }
            if modified > 0 {
                self.folder_modified_hints.put(alias.clone(), modified);
            }
            if let Some(created) = created.filter(|value| *value > 0) {
                self.folder_created_hints.put(alias, created);
            }
        }
    }
}

#[cfg(test)]
mod routing_tests;

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
