//! Keep saved shortcut identities separate from their navigable destinations.

use super::network_location_wizard::NetworkLocationWizardState;
use crate::app::state::ImageViewerApp;
use crate::domain::network_location::{network_location_path_key, NetworkLocation};
use crate::workers::file_operation_worker::FileOperationResult;
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    pub(crate) fn network_location_rename_filename(
        &self,
        path: &Path,
        requested: String,
    ) -> String {
        let alias_context = !self.global_search.active
            && (self.navigation_state.is_computer_view
                || self.context_menu.origin
                    == crate::application::context_menu::ContextMenuOrigin::SidebarTree);
        if !alias_context {
            return requested;
        }
        self.drive_state
            .network_locations
            .iter()
            .find(|location| location.matches_shortcut_path(path))
            .map(|location| location.rename_filename(&requested))
            .unwrap_or(requested)
    }

    pub(crate) fn apply_network_location_scan(&mut self, snapshot: Vec<NetworkLocation>) -> bool {
        let interaction_active = self.network_location_interaction_active();
        merge_network_location_scan(
            &mut self.drive_state.network_locations,
            snapshot,
            &mut self.network_location_wizard,
            interaction_active,
        )
    }

    pub(crate) fn resolve_network_location_path(&self, path: &Path) -> PathBuf {
        self.drive_state
            .network_locations
            .iter()
            .find(|location| location.matches_shortcut_path(path))
            .map(|location| PathBuf::from(&location.path))
            .unwrap_or_else(|| path.to_path_buf())
    }

    pub(crate) fn refresh_network_locations_after_operation(
        &mut self,
        result: &FileOperationResult,
    ) {
        let affected =
            |path: &Path| saved_shortcuts_affected(&self.drive_state.network_locations, path);
        let needs_refresh = match result {
            FileOperationResult::RenameCompleted { parent_folder, .. } => affected(parent_folder),
            FileOperationResult::DeleteCompleted { parent_folders, .. }
            | FileOperationResult::RestoreCompleted { parent_folders } => {
                parent_folders.iter().any(|path| affected(path))
            }
            FileOperationResult::CopyCompleted { dest_folder, .. } => affected(dest_folder),
            FileOperationResult::MoveCompleted {
                source_folder,
                dest_folder,
                ..
            } => affected(source_folder) || affected(dest_folder),
            FileOperationResult::MoveBatchCompleted {
                source_folders,
                dest_folder,
                ..
            } => affected(dest_folder) || source_folders.iter().any(|path| affected(path)),
            FileOperationResult::RenameBatchCompleted { .. } => {
                self.navigation_state.is_computer_view
            }
            _ => false,
        };
        if needs_refresh {
            let mut refresh_views = false;
            if let FileOperationResult::RenameCompleted {
                path,
                new_name,
                parent_folder,
            } = result
            {
                let new_path = parent_folder.join(new_name);
                for location in &mut self.drive_state.network_locations {
                    if location.matches_shortcut_path(path) {
                        location.apply_shortcut_rename(&new_path);
                        refresh_views = true;
                    }
                }
            }
            self.network_location_wizard.request_refresh(refresh_views);
        }
    }
}

fn merge_network_location_scan(
    current: &mut Vec<NetworkLocation>,
    snapshot: Vec<NetworkLocation>,
    refresh: &mut NetworkLocationWizardState,
    interaction_active: bool,
) -> bool {
    // A filesystem operation may finish while an older drive scan is still running.
    let changed = *current != snapshot;
    let Some(refresh_views) = refresh.accept_scan_result(interaction_active, changed) else {
        return false;
    };
    *current = snapshot;
    changed || refresh_views
}

fn saved_shortcuts_affected(locations: &[NetworkLocation], path: &Path) -> bool {
    let key = network_location_path_key(path);
    locations.iter().any(|location| {
        let shortcut = Path::new(&location.shortcut_path);
        network_location_path_key(shortcut) == key
            || shortcut
                .parent()
                .is_some_and(|parent| network_location_path_key(parent) == key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_scan_cannot_undo_a_committed_shortcut_rename() {
        let original = NetworkLocation {
            kind: crate::domain::network_location::NetworkLocationKind::LinkFile,
            shortcut_path: r"C:\Network Shortcuts\Old Link.lnk".into(),
            path: r"\\server\share".into(),
            label: "Old Link".into(),
        };
        let mut renamed = original.clone();
        renamed.apply_shortcut_rename(Path::new(r"C:\Network Shortcuts\New Link.lnk"));
        let mut current = vec![renamed.clone()];
        let mut refresh = NetworkLocationWizardState::default();
        refresh.request_refresh(true);
        assert!(!merge_network_location_scan(
            &mut current,
            vec![original],
            &mut refresh,
            false
        ));
        assert_eq!(current, vec![renamed.clone()]);
        assert!(!refresh.take_refresh_if_ready(true));
        assert!(refresh.take_refresh_if_ready(false));
        // Even an identical fresh snapshot must reconcile visible shortcut labels.
        assert!(merge_network_location_scan(
            &mut current,
            vec![renamed.clone()],
            &mut refresh,
            false
        ));
        assert!(!merge_network_location_scan(
            &mut current,
            vec![renamed],
            &mut refresh,
            false
        ));
    }

    #[test]
    fn changed_snapshots_wait_until_a_context_menu_or_inline_editor_closes() {
        let original = NetworkLocation {
            kind: crate::domain::network_location::NetworkLocationKind::FolderShortcut,
            shortcut_path: r"C:\Network Shortcuts\Folder".into(),
            path: r"\\server\share".into(),
            label: "Folder".into(),
        };
        let mut current = vec![original.clone()];
        let mut changed = original.clone();
        changed.apply_shortcut_rename(Path::new(r"C:\Network Shortcuts\Renamed Folder"));
        let mut refresh = NetworkLocationWizardState::default();
        assert!(!merge_network_location_scan(
            &mut current,
            vec![original.clone()],
            &mut refresh,
            true
        ));
        assert!(!refresh.take_refresh_if_ready(false));
        assert!(!merge_network_location_scan(
            &mut current,
            vec![changed.clone()],
            &mut refresh,
            true
        ));
        assert_eq!(current, vec![original]);
        assert!(refresh.take_refresh_if_ready(false));
        assert!(merge_network_location_scan(
            &mut current,
            vec![changed.clone()],
            &mut refresh,
            false
        ));
        assert_eq!(current, vec![changed]);
    }

    #[test]
    fn shortcut_updates_refresh_the_saved_list_without_treating_remote_content_as_shortcuts() {
        let locations = vec![NetworkLocation {
            kind: crate::domain::network_location::NetworkLocationKind::FolderShortcut,
            shortcut_path: r"C:\Network Shortcuts\Network Folder".into(),
            path: r"\\server\share".into(),
            label: "Network Folder".into(),
        }];
        assert!(saved_shortcuts_affected(
            &locations,
            Path::new(r"c:\network shortcuts")
        ));
        assert!(saved_shortcuts_affected(
            &locations,
            Path::new(r"C:\Network Shortcuts\Network Folder")
        ));
        assert!(!saved_shortcuts_affected(
            &locations,
            Path::new(r"\\server\share")
        ));
        assert!(!saved_shortcuts_affected(
            &locations,
            Path::new(r"C:\Other Folder")
        ));
    }
}
