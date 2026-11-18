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
    pub(crate) fn set_auto_expand_folder_tree(
        &mut self,
        enabled: bool,
        owner: RevealOwner,
        path: &str,
    ) {
        let previous_owner = self.auto_reveal_last_owner;
        self.auto_expand_folder_tree = enabled;
        self.auto_reveal_last_path.clear();
        if enabled {
            self.auto_reveal_last_path = path.to_owned();
            self.auto_reveal_last_owner = Some(owner);
            let mut tree_changed = false;
            if let Some(previous) = previous_owner.filter(|previous| {
                *previous != owner && previous.sidebar_workspace == owner.sidebar_workspace
            }) {
                tree_changed |= self.sidebar_tree.clear_tab_reveal(previous);
            }
            if self.sidebar_reveal_destination(path).is_some() {
                self.reveal_tab_path(owner, path, true, true);
                return;
            }
            tree_changed |= self.sidebar_tree.clear_tab_reveal(owner);
            if tree_changed {
                self.sync_to_tab();
                self.ui_ctx.request_repaint();
            }
        }
        self.save_preferences();
    }

    /// React only when the active location changes; never reveal the tree on
    /// every rendered frame.
    pub(crate) fn auto_reveal_active_tab(&mut self) {
        if !self.auto_expand_folder_tree || self.in_inactive_panel_context {
            return;
        }
        let path = self.navigation_state.current_path.clone();
        let owner = self.active_tree_reveal_owner();
        if path.is_empty()
            || (path == self.auto_reveal_last_path && self.auto_reveal_last_owner == Some(owner))
        {
            return;
        }
        let previous_owner = self.auto_reveal_last_owner;
        self.auto_reveal_last_path = path.clone();
        self.auto_reveal_last_owner = Some(owner);
        let owner_changed = previous_owner.is_some_and(|previous| previous != owner);
        let mut tree_changed = false;
        if let Some(previous) = previous_owner.filter(|previous| {
            *previous != owner && previous.sidebar_workspace == owner.sidebar_workspace
        }) {
            tree_changed |= self.sidebar_tree.clear_tab_reveal(previous);
        }
        if crate::domain::special_paths::is_virtual_path(&path)
            || self.sidebar_reveal_destination(&path).is_none()
        {
            tree_changed |= self.sidebar_tree.clear_tab_reveal(owner);
            if tree_changed {
                self.sync_to_tab();
                self.ui_ctx.request_repaint();
            }
            return;
        }
        self.reveal_tab_path(owner, &path, true, owner_changed);
    }

    pub(crate) fn active_tree_reveal_owner(&self) -> RevealOwner {
        let workspace_id = self.tab_manager.active().id;
        if self.dual_panel_enabled {
            if let Some(tabs) = self.tab_manager.active().dual_panel_tabs.as_ref() {
                let panel_tabs = tabs.panel(self.dual_panel_active);
                if let Some(tab) = panel_tabs.tabs.get(panel_tabs.active_tab) {
                    return RevealOwner::panel(workspace_id, tab.id).in_sidebar(workspace_id);
                }
            }
        }
        RevealOwner::workspace(workspace_id).in_sidebar(workspace_id)
    }

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
            if let Some((root, _)) = self.sidebar_reveal_destination(path) {
                self.sidebar_tree
                    .collapse_tab_tree_to(owner, &root, Path::new(path));
            }
            self.sync_to_tab();
            self.ui_ctx.request_repaint();
            return;
        }
        self.reveal_tab_path(owner, path, false, false);
    }

    fn reveal_tab_path(
        &mut self,
        owner: RevealOwner,
        path: &str,
        replace_previous: bool,
        force_same_target: bool,
    ) {
        let Some((root, section)) = self.sidebar_reveal_destination(path) else {
            return;
        };
        if replace_previous
            && !self.sidebar_tree.replace_tab_reveal(
                owner,
                &root,
                Path::new(path),
                force_same_target,
            )
        {
            return;
        }
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
        if !replace_previous {
            self.sidebar_tree
                .request_tab_reveal(owner, &root, Path::new(path));
        }
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
