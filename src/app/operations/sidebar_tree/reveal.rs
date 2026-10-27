use crate::app::state::sidebar_tree_state::reveal::{contains_path, same_path, RevealOwner};
use crate::app::ImageViewerApp;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
enum Section {
    Cloud,
    Local,
    NetworkDrive,
    NetworkLocation,
}

impl ImageViewerApp {
    fn sidebar_reveal_destination(&self, path: &str) -> Option<(PathBuf, Section)> {
        let current = Path::new(path);
        if crate::domain::special_paths::is_virtual_path(path) {
            return None;
        }
        // Prefer the most specific root, avoiding opening overlapping trees.
        let clouds = self
            .drive_state
            .cloud_roots
            .iter()
            .map(|root| (PathBuf::from(&root.path), Section::Cloud));
        let locations = self
            .drive_state
            .network_locations
            .iter()
            .map(|root| (PathBuf::from(&root.path), Section::NetworkLocation));
        let drives = self.drive_state.disks.iter().map(|(path, _)| {
            let section = if crate::ui::sidebar::get_cached_drive_type(path)
                == crate::infrastructure::windows::DriveType::Remote
            {
                Section::NetworkDrive
            } else {
                Section::Local
            };
            (PathBuf::from(path), section)
        });
        clouds
            .chain(locations)
            .chain(drives)
            .filter(|(root, _)| contains_path(root, current))
            .max_by_key(|(root, _)| root.components().count())
    }

    pub(crate) fn apply_tab_tree_action(
        &mut self,
        owner: RevealOwner,
        path: &str,
        action: crate::tabs::TabAction,
    ) {
        if action == crate::tabs::TabAction::CollapseFolderTree {
            self.sidebar_tree.undo_tab_reveal(owner, Path::new(path));
            self.sync_to_tab();
            self.ui_ctx.request_repaint();
            return;
        }
        let Some((root, section)) = self.sidebar_reveal_destination(path) else {
            return;
        };
        let before = [
            self.collapse_cloud_drives,
            self.collapse_local_disks,
            self.collapse_network_drives,
            self.collapse_network_locations,
        ];
        match section {
            Section::Cloud => self.collapse_cloud_drives = false,
            Section::Local => self.collapse_local_disks = false,
            Section::NetworkDrive => self.collapse_network_drives = false,
            Section::NetworkLocation => self.collapse_network_locations = false,
        }
        self.sidebar_tree
            .request_tab_reveal(owner, &root, Path::new(path));
        let after = [
            self.collapse_cloud_drives,
            self.collapse_local_disks,
            self.collapse_network_drives,
            self.collapse_network_locations,
        ];
        crate::ui::app::panels::restore_sidebar_drives_height(
            before,
            after,
            &mut self.layout.sidebar_drives_height,
            &mut self.layout.sidebar_drives_heights,
        );
        self.layout.set_sidebar_sections_collapsed(false);
        self.sync_to_tab();
        self.save_preferences();
        self.ui_ctx.request_repaint();
    }

    /// A pending reveal may belong to an inactive tab. Validate its owner,
    /// not the path displayed by the currently active content pane.
    pub(crate) fn validate_tab_tree_reveals(&mut self) {
        self.sidebar_tree
            .set_reveal_sidebar_workspace(self.tab_manager.active().id);
        let active = self.tab_manager.active_tab;
        let mut valid = Vec::new();
        for (index, tab) in self.tab_manager.tabs.iter().enumerate() {
            let path = if index == active {
                &self.navigation_state.current_path
            } else {
                &tab.path
            };
            valid.push((RevealOwner::workspace(tab.id), PathBuf::from(path)));
            if let Some(panels) = &tab.dual_panel_tabs {
                for (panel, set) in [
                    (crate::app::dual_panel::ActivePanel::Left, &panels.left),
                    (crate::app::dual_panel::ActivePanel::Right, &panels.right),
                ] {
                    for (inner_index, inner) in set.tabs.iter().enumerate() {
                        let path = if index == active
                            && self.dual_panel_enabled
                            && inner_index == set.active_tab
                        {
                            if panel == self.dual_panel_active {
                                &self.navigation_state.current_path
                            } else {
                                self.dual_panel_inactive_state
                                    .as_ref()
                                    .map(|state| &state.path)
                                    .unwrap_or(&inner.snapshot.path)
                            }
                        } else {
                            &inner.snapshot.path
                        };
                        valid.push((RevealOwner::panel(tab.id, inner.id), PathBuf::from(path)));
                    }
                }
            }
        }
        self.sidebar_tree.retain_tab_reveals(|owner, path| {
            self.tab_manager
                .tabs
                .iter()
                .any(|tab| tab.id == owner.sidebar_workspace)
                && valid.iter().any(|(candidate, current)| {
                    candidate.workspace == owner.workspace
                        && candidate.panel_tab == owner.panel_tab
                        && same_path(current, path)
                })
        });
    }
}
