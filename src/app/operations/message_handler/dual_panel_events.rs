use crate::app::state::ImageViewerApp;
use std::path::{Path, PathBuf};

fn renamed_path_for_candidate(
    candidate: &Path,
    old_path: &Path,
    new_path: &Path,
) -> Option<PathBuf> {
    let candidate_clean = ImageViewerApp::clean_path(candidate);
    let old_clean = ImageViewerApp::clean_path(old_path);
    let new_clean = ImageViewerApp::clean_path(new_path);

    if ImageViewerApp::normalize_for_match(&candidate_clean)
        == ImageViewerApp::normalize_for_match(&old_clean)
    {
        return Some(new_clean);
    }

    let candidate_components: Vec<_> = candidate_clean.components().collect();
    let old_components: Vec<_> = old_clean.components().collect();

    if candidate_components.len() <= old_components.len() {
        return None;
    }

    let starts_with_old = old_components.iter().zip(candidate_components.iter()).all(
        |(old_component, candidate_component)| {
            old_component.as_os_str().to_string_lossy().to_lowercase()
                == candidate_component
                    .as_os_str()
                    .to_string_lossy()
                    .to_lowercase()
        },
    );

    if !starts_with_old {
        return None;
    }

    let mut renamed = new_clean;
    for component in candidate_components.iter().skip(old_components.len()) {
        renamed.push(component.as_os_str());
    }
    Some(renamed)
}

impl ImageViewerApp {
    /// Reload the inactive dual panel if its folder matches any of the given paths.
    /// Used when file operations or external watcher events may have affected
    /// the inactive panel's folder contents.
    pub(super) fn reload_inactive_panel_if_matches(&mut self, folders: &[&PathBuf]) {
        if !self.dual_panel_enabled {
            return;
        }
        let inactive_path = match self.dual_panel_inactive_state.as_ref() {
            Some(s) => s.path.clone(),
            None => return,
        };
        let inactive_norm = Self::normalize_for_match(Path::new(&inactive_path));

        let matches = folders
            .iter()
            .any(|f| Self::normalize_for_match(f.as_path()) == inactive_norm);
        if !matches {
            return;
        }

        log::info!(
            "[DualPanel] Inactive panel folder affected by change, reloading: {}",
            inactive_path
        );

        let inactive_pb = PathBuf::from(&inactive_path);
        self.directory_dirty_registry.mark_dirty(&inactive_pb);
        self.directory_cache.invalidate(&inactive_pb);
        if let Some(ref di) = self.directory_index {
            let _ = di.invalidate(&inactive_pb);
        }

        self.with_inactive_panel(|app| {
            app.loaded_path.clear();
            app.load_folder_for_inactive();
        });
    }

    pub(super) fn apply_rename_to_inactive_panel_if_affected(
        &mut self,
        old_path: &Path,
        new_path: &Path,
    ) {
        if !self.dual_panel_enabled {
            return;
        }

        let Some(inactive_path) = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|snapshot| PathBuf::from(&snapshot.path))
        else {
            return;
        };

        let old_clean = Self::clean_path(old_path);
        let new_clean = Self::clean_path(new_path);
        let inactive_norm = Self::normalize_for_match(&inactive_path);
        let inactive_is_renamed_path =
            renamed_path_for_candidate(&inactive_path, &old_clean, &new_clean);

        let inactive_shows_rename_parent = [old_clean.parent(), new_clean.parent()]
            .into_iter()
            .flatten()
            .any(|parent| Self::normalize_for_match(parent) == inactive_norm);

        if inactive_is_renamed_path.is_none() && !inactive_shows_rename_parent {
            return;
        }

        if let Some(parent) = old_clean.parent() {
            self.invalidate_directory_caches(parent);
        }
        if let Some(parent) = new_clean.parent() {
            self.invalidate_directory_caches(parent);
        }

        log::info!(
            "[DualPanel] Inactive panel affected by external rename: {} -> {}",
            old_clean.display(),
            new_clean.display()
        );

        self.with_inactive_panel(|app| {
            if let Some(renamed_path) = inactive_is_renamed_path {
                let renamed_path_string = renamed_path.to_string_lossy().to_string();
                app.navigation_state.current_path = renamed_path_string.clone();
                app.navigation_state.path_input = renamed_path_string.clone();
                if let Some(current_history_path) = app
                    .navigation_state
                    .navigation
                    .paths
                    .get_mut(app.navigation_state.navigation.current_index)
                {
                    *current_history_path = renamed_path_string;
                }

                app.loaded_path.clear();
                app.items = std::sync::Arc::new(Vec::new());
                app.all_items.clear();
                app.selected_item = None;
                app.selected_file = None;
                app.selected_thumbnail = None;
                app.selected_metadata = None;
                app.multi_selection.clear();
                app.selection_anchor = None;
                app.load_folder_for_inactive();
            } else if inactive_shows_rename_parent
                && !app.try_apply_rename_to_ui(&old_clean, &new_clean)
            {
                app.loaded_path.clear();
                app.load_folder_for_inactive();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_path_for_candidate_matches_exact_folder_case_insensitively() {
        let renamed = renamed_path_for_candidate(
            Path::new(r"D:\Old"),
            Path::new(r"d:\old"),
            Path::new(r"D:\New"),
        )
        .expect("exact renamed folder should translate");

        assert_eq!(
            ImageViewerApp::normalize_for_match(&renamed),
            ImageViewerApp::normalize_for_match(Path::new(r"D:\New"))
        );
    }

    #[test]
    fn renamed_path_for_candidate_preserves_descendant_suffix() {
        let renamed = renamed_path_for_candidate(
            Path::new(r"D:\Old\Child\Leaf"),
            Path::new(r"D:\Old"),
            Path::new(r"D:\New"),
        )
        .expect("descendant of renamed folder should translate");

        assert_eq!(
            ImageViewerApp::normalize_for_match(&renamed),
            ImageViewerApp::normalize_for_match(Path::new(r"D:\New\Child\Leaf"))
        );
    }

    #[test]
    fn renamed_path_for_candidate_ignores_unrelated_path() {
        assert!(renamed_path_for_candidate(
            Path::new(r"D:\Other"),
            Path::new(r"D:\Old"),
            Path::new(r"D:\New"),
        )
        .is_none());
    }
}
