//! Persistence and workspace actions for saved tab groups.

use crate::app::ImageViewerApp;
use crate::domain::folder_group::{SavedGroupPanelTabs, SavedGroupTab, SavedGroupTabs};
use crate::tabs::{PanelTab, PanelTabSet, TabState};

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
        self.save_group_name = self.tab_manager.active().title.trim().to_owned();
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
        if !self
            .tab_manager
            .can_replace_unpinned_tabs_with(saved.tabs.len())
        {
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

        let pinned_count = self
            .tab_manager
            .tabs
            .iter()
            .filter(|tab| tab.pinned)
            .count();
        if !self.tab_manager.replace_unpinned_tabs_with_states(restored) {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        }
        self.tab_manager.active_tab = pinned_count + saved.active_tab.min(saved.tabs.len() - 1);
        self.tab_manager.restore_tag_titles(&self.tag_definitions);

        self.sync_from_tab();
        self.update_video_visibility();
        self.save_preferences();
    }

    fn runtime_tab_from_group(&mut self, saved: &SavedGroupTab) -> TabState {
        let path = self.resolve_group_path(&saved.path);
        let mut tab = TabState::new_at_path(0, &path);
        tab.view_mode = self.new_tab_view_mode();
        tab.sort_mode = self.sort_mode;
        tab.sort_descending = self.sort_descending;
        tab.folders_position = self.folders_position;
        tab.show_left_sidebar = self.show_left_sidebar;
        tab.show_preview_panel = self.show_preview_panel;
        tab.collapse_tags = self.collapse_tags;
        tab.group_mode = self.group_mode;
        tab.group_descending = self.group_descending;

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
            let snapshot = self.new_panel_tab_snapshot(&path);
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
        if !self.tab_manager.can_replace_unpinned_tabs_with(paths.len()) {
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
        if !self.tab_manager.replace_unpinned_tabs_with(&paths) {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        }
        for tab in self.tab_manager.tabs.iter_mut().filter(|tab| !tab.pinned) {
            tab.view_mode = view_mode;
            tab.sort_mode = sort_mode;
            tab.sort_descending = sort_descending;
            tab.folders_position = folders_position;
        }
        self.sync_from_tab();
        self.update_video_visibility();
        self.save_preferences();
    }

    fn resolve_group_path(&self, path: &str) -> String {
        self.resolve_network_location_path(std::path::Path::new(path))
            .to_string_lossy()
            .into_owned()
    }
}
