//! Persistence and workspace actions for saved tab groups.

use crate::app::ImageViewerApp;
use crate::domain::folder_group::{SavedGroupPanelTabs, SavedGroupTab, SavedGroupTabs};
use crate::tabs::{PanelTab, PanelTabSet, TabState};

fn group_tab_display_title(
    path: &str,
    tag_definitions: &rustc_hash::FxHashMap<i64, crate::domain::file_tag::FileTag>,
) -> String {
    match path {
        crate::domain::special_paths::COMPUTER_VIEW_ID => rust_i18n::t!("nav.computer").to_string(),
        crate::domain::special_paths::RECYCLE_BIN_VIEW_ID => {
            rust_i18n::t!("nav.recycle_bin").to_string()
        }
        _ => {
            if let Some(tag_id) = crate::domain::special_paths::tag_id_from_view_path(path) {
                crate::app::operations::tag_ops::tag_view_title(tag_id, tag_definitions)
            } else {
                std::path::Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_owned())
            }
        }
    }
}

impl ImageViewerApp {
    pub(crate) fn workspace_tab_paths(&self) -> Vec<String> {
        self.tab_manager
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                if index == self.tab_manager.active_tab {
                    self.navigation_state.current_path.clone()
                } else {
                    tab.path.clone()
                }
            })
            .collect()
    }

    pub(crate) fn begin_save_tabs_as_group(&mut self) {
        if self.show_save_group_dialog || self.tab_navigation_blocked() {
            return;
        }
        self.sync_to_tab();
        self.save_group_name = self
            .display_title_for_group_path(&self.navigation_state.current_path)
            .trim()
            .to_owned();
        if self.save_group_name.is_empty() {
            self.save_group_name = rust_i18n::t!("tabs.group_default_name").to_string();
        }
        self.show_save_group_dialog = true;
    }

    pub(crate) fn save_tabs_as_group(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }

        // Capture the live active panel before serializing the workspace.
        self.sync_to_tab();
        let saved_tabs = self.tab_manager.saved_group_tabs();
        let paths: Vec<String> = saved_tabs.tabs.iter().map(|tab| tab.path.clone()).collect();
        if paths.is_empty() {
            return;
        }

        match self
            .app_state_db
            .save_folder_group(name, &paths, Some(&saved_tabs))
        {
            Ok(()) => {
                self.folder_groups = self.app_state_db.get_all_folder_groups();
                self.show_save_group_dialog = false;
                self.save_group_name.clear();
                self.ui_ctx.request_repaint();
            }
            Err(_) => self
                .notifications
                .warning(rust_i18n::t!("tabs.group_save_failed").to_string()),
        }
    }

    pub(crate) fn open_folder_group(&mut self, group_id: i64) {
        if self.tab_navigation_blocked() {
            return;
        }
        let Some(group) = self
            .folder_groups
            .iter()
            .find(|group| group.id == group_id)
            .cloned()
        else {
            return;
        };

        match group
            .saved_tabs
            .as_ref()
            .filter(|saved| !saved.tabs.is_empty())
        {
            Some(saved) => self.open_saved_group_tabs(saved),
            None => self.open_group_paths(&group.paths),
        }
    }

    pub(crate) fn delete_folder_group(&mut self, group_id: i64) {
        match self.app_state_db.delete_folder_group(group_id) {
            Ok(()) => {
                self.folder_groups = self.app_state_db.get_all_folder_groups();
                self.ui_ctx.request_repaint();
            }
            Err(_) => self
                .notifications
                .warning(rust_i18n::t!("sidebar.group_delete_failed").to_string()),
        }
    }

    fn open_saved_group_tabs(&mut self, saved: &SavedGroupTabs) {
        if !self.tab_manager.can_append_tabs(saved.tabs.len()) {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        }

        self.sync_to_tab();
        let mut restored = Vec::with_capacity(saved.tabs.len());
        for saved_tab in &saved.tabs {
            restored.push(self.runtime_tab_from_group(saved_tab));
        }

        let Some(first_index) = self.tab_manager.append_tabs_with_states(restored) else {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        };
        self.tab_manager.active_tab = first_index + saved.active_tab.min(saved.tabs.len() - 1);
        self.tab_manager.restore_tag_titles(&self.tag_definitions);

        self.sync_from_tab();
        self.update_video_visibility();
        self.save_preferences();
    }

    fn runtime_tab_from_group(&mut self, saved: &SavedGroupTab) -> TabState {
        let path = self.resolve_group_path(&saved.path);
        let mut tab = TabState::new_at_path(0, &path);
        tab.sort_mode = self.sort_mode;
        tab.sort_descending = self.sort_descending;
        tab.folders_position = self.folders_position;
        tab.show_left_sidebar = self.show_left_sidebar;
        tab.show_preview_panel = self.show_preview_panel;
        tab.collapse_tags = self.collapse_tags;
        tab.group_mode = self.group_mode;
        tab.group_descending = self.group_descending;
        let fallback_view_mode = self.new_tab_view_mode();
        tab.view_mode = self
            .group_restore_view_mode(&path, saved.view_mode.as_deref(), fallback_view_mode)
            .0;
        if let Some(resolved) = self.folder_lock_resolution_for_path(&path) {
            tab.sort_mode = resolved.lock.sort_mode;
            tab.sort_descending = resolved.lock.sort_descending;
            tab.folders_position = resolved.lock.folders_position;
            tab.group_mode = resolved.lock.group_mode;
            tab.group_descending = resolved.lock.group_descending;
        }

        if let Some(dual) = &saved.dual_panel {
            let left = self.runtime_group_panel_tabs(&dual.left, 0);
            let right = self.runtime_group_panel_tabs(&dual.right, left.tabs.len());
            tab.restore_saved_group_dual_panel(dual, left, right);
        }

        tab
    }

    fn runtime_group_panel_tabs(
        &mut self,
        saved: &SavedGroupPanelTabs,
        id_offset: usize,
    ) -> PanelTabSet {
        let mut tabs = Vec::with_capacity(saved.tabs.len());
        for (index, saved_tab) in saved.tabs.iter().enumerate() {
            let path = self.resolve_group_path(&saved_tab.path);
            let mut snapshot = self.new_panel_tab_snapshot(&path);
            let (view_mode, current_folder_locked) = self.group_restore_view_mode(
                &path,
                saved_tab.view_mode.as_deref(),
                snapshot.view_mode,
            );
            snapshot.view_mode = view_mode;
            snapshot.current_folder_locked = current_folder_locked;
            if let Some(resolved) = self.folder_lock_resolution_for_path(&path) {
                snapshot.sort_mode = resolved.lock.sort_mode;
                snapshot.sort_descending = resolved.lock.sort_descending;
                snapshot.folders_position = resolved.lock.folders_position;
                snapshot.group_mode = resolved.lock.group_mode;
                snapshot.group_descending = resolved.lock.group_descending;
            }
            tabs.push(PanelTab {
                id: id_offset + index,
                pinned: saved_tab.pinned,
                snapshot,
            });
        }
        let active_tab = saved.active.min(tabs.len().saturating_sub(1));
        PanelTabSet { tabs, active_tab }
    }

    fn open_group_paths(&mut self, paths: &[String]) {
        if !self.tab_manager.can_append_tabs(paths.len()) {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        }

        let paths: Vec<_> = paths
            .iter()
            .map(|path| self.resolve_group_path(path))
            .collect();
        self.sync_to_tab();
        let view_mode = self.new_tab_view_mode();
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let folders_position = self.folders_position;
        let group_mode = self.group_mode;
        let group_descending = self.group_descending;
        let show_left_sidebar = self.show_left_sidebar;
        let show_preview_panel = self.show_preview_panel;
        let collapse_tags = self.collapse_tags;
        let mut tabs: Vec<_> = paths
            .iter()
            .map(|path| {
                let mut tab = TabState::new_at_path(0, path);
                tab.view_mode = view_mode;
                tab.sort_mode = sort_mode;
                tab.sort_descending = sort_descending;
                tab.folders_position = folders_position;
                tab.group_mode = group_mode;
                tab.group_descending = group_descending;
                tab.show_left_sidebar = show_left_sidebar;
                tab.show_preview_panel = show_preview_panel;
                tab.collapse_tags = collapse_tags;
                tab
            })
            .collect();
        for tab in &mut tabs {
            if let Some(resolved) = self.folder_lock_resolution_for_path(&tab.path) {
                tab.view_mode = resolved.lock.view_mode;
                tab.sort_mode = resolved.lock.sort_mode;
                tab.sort_descending = resolved.lock.sort_descending;
                tab.folders_position = resolved.lock.folders_position;
                tab.group_mode = resolved.lock.group_mode;
                tab.group_descending = resolved.lock.group_descending;
            }
        }
        let Some(first_index) = self.tab_manager.append_tabs_with_states(tabs) else {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        };
        self.tab_manager.active_tab = first_index;
        self.tab_manager.restore_tag_titles(&self.tag_definitions);
        self.sync_from_tab();
        self.update_video_visibility();
        self.save_preferences();
    }

    fn resolve_group_path(&self, path: &str) -> String {
        self.resolve_network_location_path(std::path::Path::new(path))
            .to_string_lossy()
            .into_owned()
    }

    fn display_title_for_group_path(&self, path: &str) -> String {
        group_tab_display_title(path, &self.tag_definitions)
    }
}

#[cfg(test)]
mod tests {
    use super::group_tab_display_title;

    #[test]
    fn computer_view_name_uses_its_localized_display_title() {
        let title = group_tab_display_title(
            crate::domain::special_paths::COMPUTER_VIEW_ID,
            &rustc_hash::FxHashMap::default(),
        );

        assert_eq!(title, rust_i18n::t!("nav.computer").to_string());
        assert_ne!(title, crate::domain::special_paths::COMPUTER_VIEW_ID);
    }
}
