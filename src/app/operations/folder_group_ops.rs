//! Persistence and workspace actions for saved tab groups.

use crate::app::ImageViewerApp;

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
        let paths = self.workspace_tab_paths();
        if name.is_empty() || paths.is_empty() {
            return;
        }

        match self.app_state_db.save_folder_group(name, &paths) {
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
        let Some(paths) = self
            .folder_groups
            .iter()
            .find(|group| group.id == group_id)
            .map(|group| group.paths.clone())
        else {
            return;
        };
        if !self.tab_manager.can_replace_unpinned_tabs_with(paths.len()) {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        }

        let paths: Vec<_> = paths
            .iter()
            .map(|path| {
                self.resolve_network_location_path(std::path::Path::new(path))
                    .to_string_lossy()
                    .into_owned()
            })
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
}
