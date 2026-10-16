use crate::app::{dual_panel::ActivePanel, ImageViewerApp};

impl ImageViewerApp {
    pub(crate) fn open_location_in_new_tab(&mut self, panel: ActivePanel, path: &str) {
        if self.navigation_state.show_settings_window
            || self.global_search.active
            || self.context_menu.is_open
            || self.show_tag_manager
            || self.batch_rename_state.is_some()
            || self.renaming_state.is_some()
            || self.sidebar_renaming.is_some()
            || self.sidebar_folder_rename.is_some()
            || self.is_item_dragging
            || self.file_panel_input_blocked_by_drag_move_confirmation()
        {
            return;
        }
        if self.dual_panel_enabled {
            self.dual_panel_add_tab_at(panel, path);
            return;
        }
        if !self.tab_manager.can_add_tab() {
            self.notifications.warning(
                rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
            );
            return;
        }
        self.sync_to_tab();
        let view_mode = self.view_mode;
        let sort_mode = self.sort_mode;
        let sort_descending = self.sort_descending;
        let folders_position = self.folders_position;
        if !self.tab_manager.new_tab_at(path) {
            return;
        }
        let active = self.tab_manager.active_mut();
        active.view_mode = view_mode;
        active.sort_mode = sort_mode;
        active.sort_descending = sort_descending;
        active.folders_position = folders_position;
        self.sync_from_tab();
        // sync_from_tab already prepares Computer/tag views and starts regular
        // folder loads. An empty Recycle Bin tab needs its own virtual listing.
        if self.navigation_state.is_recycle_bin_view {
            self.setup_recycle_bin_view();
        }
        self.sync_to_tab();
        self.update_video_visibility();
        self.save_preferences();
    }
}
