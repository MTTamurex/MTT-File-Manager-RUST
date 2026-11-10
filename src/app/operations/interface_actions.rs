use crate::app::{navigation_state::SettingsSection, ImageViewerApp};

impl ImageViewerApp {
    pub(crate) fn open_settings(&mut self, section: SettingsSection) {
        self.navigation_state.active_settings_section = section;
        self.navigation_state.show_settings_window = true;
    }

    pub(crate) fn toggle_left_sidebar(&mut self) {
        self.show_left_sidebar = !self.show_left_sidebar;
        self.tab_manager.active_mut().show_left_sidebar = self.show_left_sidebar;
        self.save_preferences();
    }

    pub(crate) fn toggle_preview_panel(&mut self) {
        self.show_preview_panel = !self.show_preview_panel;
        self.tab_manager.active_mut().show_preview_panel = self.show_preview_panel;
        if self.show_preview_panel && self.needs_selected_preview_preparation() {
            self.update_selected_thumbnail();
        }
        self.update_video_visibility();
    }
}
