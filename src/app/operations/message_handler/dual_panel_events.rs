use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
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

fn parent_path_for_deleted_panel(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    if parent.as_os_str().is_empty() {
        None
    } else {
        Some(parent.to_string_lossy().to_string())
    }
}

/// Normalized folder key tolerant of trailing separators, used to match a
/// panel path against folders affected by file operations. `Path::parent()`
/// of `C:\x\a.jpg` yields `C:\x` while a drive-root parent yields `C:\`;
/// panel paths may carry either form, so trailing separators are ignored.
fn folder_key_for_match(path: &Path) -> String {
    ImageViewerApp::normalize_for_match(path)
        .trim_end_matches(['\\', '/'])
        .to_string()
}

fn folder_path_for_load(path: &str) -> PathBuf {
    if path.len() == 2 && path.ends_with(':') {
        PathBuf::from(format!("{path}\\"))
    } else {
        PathBuf::from(path)
    }
}

fn inactive_panel_reload_target(
    dual_panel_enabled: bool,
    inactive_path: &str,
    affected_folders: &[&PathBuf],
) -> Option<PathBuf> {
    if !dual_panel_enabled {
        return None;
    }

    let inactive_norm = folder_key_for_match(Path::new(inactive_path));
    affected_folders
        .iter()
        .find(|folder| folder_key_for_match(folder) == inactive_norm)
        .map(|_| folder_path_for_load(inactive_path))
}

fn path_is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    let candidate_clean = ImageViewerApp::clean_path(candidate);
    let root_clean = ImageViewerApp::clean_path(root);

    if ImageViewerApp::normalize_for_match(&candidate_clean)
        == ImageViewerApp::normalize_for_match(&root_clean)
    {
        return true;
    }

    let candidate_components: Vec<_> = candidate_clean.components().collect();
    let root_components: Vec<_> = root_clean.components().collect();

    candidate_components.len() > root_components.len()
        && root_components.iter().zip(candidate_components.iter()).all(
            |(root_component, candidate_component)| {
                root_component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&candidate_component.as_os_str().to_string_lossy())
            },
        )
}

impl ImageViewerApp {
    pub(super) fn inactive_panel_path_matches(&self, path: &Path) -> bool {
        if !self.dual_panel_enabled {
            return false;
        }

        let Some(snapshot) = self.dual_panel_inactive_state.as_ref() else {
            return false;
        };
        if snapshot.is_computer_view
            || snapshot.is_recycle_bin_view
            || crate::domain::special_paths::is_virtual_path(&snapshot.path)
        {
            return false;
        }

        Self::normalize_for_match(Path::new(&snapshot.path)) == Self::normalize_for_match(path)
    }

    pub(super) fn navigate_inactive_panel_to_parent_after_vanished(
        &mut self,
        vanished_path: &Path,
    ) -> bool {
        if !self.inactive_panel_path_matches(vanished_path) {
            return false;
        }

        self.navigate_inactive_panel_to_parent_of_deleted_path(vanished_path)
    }

    pub(super) fn navigate_inactive_panel_after_deleted_paths(
        &mut self,
        deleted_paths: &[PathBuf],
    ) -> bool {
        if !self.dual_panel_enabled {
            return false;
        }

        let Some(inactive_path) = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|snapshot| PathBuf::from(&snapshot.path))
        else {
            return false;
        };

        let Some(deleted_root) = deleted_paths
            .iter()
            .find(|deleted_path| path_is_same_or_descendant(&inactive_path, deleted_path))
            .cloned()
        else {
            return false;
        };

        self.navigate_inactive_panel_to_parent_of_deleted_path(&deleted_root)
    }

    fn navigate_inactive_panel_to_parent_of_deleted_path(&mut self, deleted_path: &Path) -> bool {
        if !self.dual_panel_enabled {
            return false;
        }

        let target_parent = parent_path_for_deleted_panel(deleted_path);
        if let Some(parent) = target_parent.as_deref().map(PathBuf::from) {
            self.directory_dirty_registry.mark_dirty(&parent);
            self.directory_cache.invalidate(&parent);
            if let Some(ref di) = self.directory_index {
                let _ = di.invalidate(&parent);
            }
        }

        log::warn!(
            "[DualPanel] Inactive panel folder vanished: {}",
            deleted_path.display()
        );

        self.with_inactive_panel(|app| {
            app.loaded_path.clear();
            app.items = std::sync::Arc::new(Vec::new());
            app.group_projection = std::sync::Arc::new(Default::default());
            app.all_items_mut().clear();
            app.selected_item = None;
            app.selected_file = None;
            app.selected_thumbnail = None;
            app.selected_metadata = None;
            app.multi_selection.clear();
            app.selection_anchor = None;

            if let Some(parent_path) = target_parent {
                app.navigation_state
                    .navigation
                    .navigate_to(parent_path.clone());
                app.navigation_state.current_path = parent_path.clone();
                app.navigation_state.path_input = parent_path;
                app.navigation_state.is_computer_view = false;
                app.navigation_state.is_recycle_bin_view = false;
                app.apply_folder_lock_if_present();
                app.load_folder_for_inactive();
            } else {
                app.navigation_state
                    .navigation
                    .navigate_to(COMPUTER_VIEW_ID.to_string());
                app.setup_computer_view();
            }
        });

        true
    }

    /// Reload the inactive dual panel if its folder matches any of the given paths.
    /// Used when file operations or external watcher events may have affected
    /// the inactive panel's folder contents.
    pub(in crate::app::operations) fn reload_inactive_panel_if_matches(
        &mut self,
        folders: &[&PathBuf],
    ) {
        let Some(inactive_path) = self
            .dual_panel_inactive_state
            .as_ref()
            .map(|snapshot| snapshot.path.clone())
        else {
            return;
        };
        let Some(reload_folder) =
            inactive_panel_reload_target(self.dual_panel_enabled, &inactive_path, folders)
        else {
            return;
        };

        log::info!(
            "[DualPanel] Inactive panel folder affected by change, reloading: {}",
            inactive_path
        );

        self.directory_dirty_registry.mark_dirty(&reload_folder);
        self.directory_cache.invalidate(&reload_folder);
        if let Some(ref di) = self.directory_index {
            let _ = di.invalidate(&reload_folder);
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
                app.group_projection = std::sync::Arc::new(Default::default());
                app.all_items_mut().clear();
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

    #[test]
    fn parent_path_for_deleted_panel_returns_parent_folder() {
        assert_eq!(
            parent_path_for_deleted_panel(Path::new(r"D:\Teste")),
            Some(r"D:\".to_string())
        );
    }

    #[test]
    fn path_is_same_or_descendant_matches_descendant_case_insensitively() {
        assert!(path_is_same_or_descendant(
            Path::new(r"D:\Teste\Sub"),
            Path::new(r"d:\teste")
        ));
    }

    #[test]
    fn path_is_same_or_descendant_rejects_common_prefix_sibling() {
        assert!(!path_is_same_or_descendant(
            Path::new(r"D:\Teste 10"),
            Path::new(r"D:\Teste")
        ));
    }

    #[test]
    fn folder_key_for_match_ignores_trailing_separator() {
        assert_eq!(
            folder_key_for_match(Path::new(r"D:\Images\")),
            folder_key_for_match(Path::new(r"d:\images"))
        );
    }

    #[test]
    fn folder_key_for_match_equates_drive_root_forms() {
        assert_eq!(
            folder_key_for_match(Path::new(r"D:")),
            folder_key_for_match(Path::new(r"D:\"))
        );
    }

    #[test]
    fn folder_key_for_match_ignores_extended_path_prefix() {
        assert_eq!(
            folder_key_for_match(Path::new(r"\\?\D:\Images")),
            folder_key_for_match(Path::new(r"D:\Images\"))
        );
    }

    #[test]
    fn folder_key_for_match_keeps_distinct_folders_distinct() {
        assert_ne!(
            folder_key_for_match(Path::new(r"D:\Images")),
            folder_key_for_match(Path::new(r"D:\Images 2"))
        );
    }

    #[test]
    fn move_completion_routes_reload_to_inactive_destination_without_focus_change() {
        let source = PathBuf::from(r"D:\Source");
        let destination = PathBuf::from(r"E:\Destination");
        let affected_folders = [&source, &destination];

        let reload_target =
            inactive_panel_reload_target(true, r"E:\Destination\", &affected_folders);

        assert_eq!(reload_target, Some(PathBuf::from(r"E:\Destination\")));
    }

    #[test]
    fn move_completion_does_not_route_reload_when_dual_panel_is_disabled() {
        let source = PathBuf::from(r"D:\Source");
        let destination = PathBuf::from(r"E:\Destination");
        let affected_folders = [&source, &destination];

        let reload_target =
            inactive_panel_reload_target(false, r"E:\Destination", &affected_folders);

        assert_eq!(reload_target, None);
    }

    #[test]
    fn reload_target_uses_the_inactive_panel_path_spelling() {
        let operation_path = PathBuf::from(r"\\?\x:\18\");
        let affected_folders = [&operation_path];

        let reload_target = inactive_panel_reload_target(true, r"X:\18", &affected_folders);

        assert_eq!(reload_target, Some(PathBuf::from(r"X:\18")));
    }

    #[test]
    fn reload_target_expands_drive_root_for_folder_loading() {
        let operation_path = PathBuf::from(r"X:\");
        let affected_folders = [&operation_path];

        let reload_target = inactive_panel_reload_target(true, r"X:", &affected_folders);

        assert_eq!(reload_target, Some(PathBuf::from(r"X:\")));
    }
}
