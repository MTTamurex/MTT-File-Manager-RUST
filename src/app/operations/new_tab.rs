use crate::app::{dual_panel::ActivePanel, ImageViewerApp};

mod batch;

impl ImageViewerApp {
    /// Apply the archive preference only to item activation, never to history,
    /// address-bar navigation or explicit requests for an existing tab.
    pub(crate) fn activate_location(&mut self, path: &str) {
        let destination = self.resolve_network_location_path(std::path::Path::new(path));
        let path = destination.to_string_lossy();
        let path = path.as_ref();
        if self.tab_preferences.archive_activation_candidate(path) {
            self.queue_archive_activation(path);
        } else {
            self.navigate_to(path);
        }
    }

    pub(crate) fn create_default_tab(&mut self) {
        if self.tab_navigation_blocked() {
            return;
        }
        let path = self.tab_preferences.location.resolve(
            &self.navigation_state.current_path,
            &self.tab_preferences.custom_path,
        );
        // Ctrl belongs to the New Tab shortcut itself, not to a focus override.
        self.open_locations_in_new_tabs(self.dual_panel_active, &[path], false);
    }

    pub(crate) fn create_default_main_tab(&mut self) {
        let path = self.tab_preferences.location.resolve(
            &self.navigation_state.current_path,
            &self.tab_preferences.custom_path,
        );
        self.open_location_in_main_tab(&path);
    }

    pub(crate) fn open_location_in_new_tab(&mut self, panel: ActivePanel, path: &str) {
        if self.tab_navigation_blocked() {
            return;
        }
        let ctrl = self.ui_ctx.input(|input| input.modifiers.ctrl);
        self.open_locations_in_new_tabs(panel, &[path.to_owned()], ctrl);
    }

    fn open_location_in_main_tab(&mut self, path: &str) {
        if self.tab_navigation_blocked() {
            return;
        }
        let ctrl = self.ui_ctx.input(|input| input.modifiers.ctrl);
        self.open_main_tabs(&[path.to_owned()], ctrl);
    }

    pub(crate) fn tab_navigation_blocked(&self) -> bool {
        self.navigation_state.show_settings_window
            || self.folder_export.is_some()
            || self.global_search.active
            || self.context_menu.is_open
            || self.show_tag_manager
            || self.batch_rename_state.is_some()
            || self.renaming_state.is_some()
            || self.sidebar_renaming.is_some()
            || self.sidebar_folder_rename.is_some()
            || self.is_item_dragging
            || self.file_panel_input_blocked_by_drag_move_confirmation()
    }
}
