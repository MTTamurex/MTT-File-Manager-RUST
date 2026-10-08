use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::shell_integration;
use rust_i18n::t;

impl ImageViewerApp {
    pub fn apply_shell_integration_setting_changes(
        &mut self,
        default_file_manager_changed: bool,
        context_menu_changed: bool,
    ) {
        if default_file_manager_changed {
            self.apply_default_file_manager_setting();
        }
        if context_menu_changed {
            self.apply_context_menu_setting();
        }
    }

    fn persist_shell_integration_preferences(&mut self) -> bool {
        self.save_preferences();
        self.force_save_preferences()
    }

    fn apply_default_file_manager_setting(&mut self) {
        let enabled = self.use_mtt_as_default_file_manager;
        if !self.persist_shell_integration_preferences() {
            self.use_mtt_as_default_file_manager = !enabled;
            self.persist_shell_integration_preferences();
            self.notifications
                .error(t!("settings.shell_integration_save_failed").to_string());
            return;
        }

        if let Err(error) = shell_integration::set_default_file_manager_enabled_for_current_user(
            &self.app_state_db,
            enabled,
        ) {
            self.use_mtt_as_default_file_manager = !enabled;
            if !self.persist_shell_integration_preferences() {
                log::error!(
                    "[SHELL-INTEGRATION] Could not persist the reverted default-handler setting"
                );
            }
            self.notifications
                .error(t!("settings.shell_integration_failed", error = error).to_string());
        }
    }

    fn apply_context_menu_setting(&mut self) {
        let enabled = self.add_open_in_mtt_context_menu;
        if !self.persist_shell_integration_preferences() {
            self.add_open_in_mtt_context_menu = !enabled;
            self.persist_shell_integration_preferences();
            self.notifications
                .error(t!("settings.shell_integration_save_failed").to_string());
            return;
        }

        let label = t!("context_menu.open_in_mtt").to_string();
        if let Err(error) = shell_integration::set_open_in_mtt_context_menu_enabled(enabled, &label)
        {
            self.add_open_in_mtt_context_menu = !enabled;
            if !self.persist_shell_integration_preferences() {
                log::error!(
                    "[SHELL-INTEGRATION] Could not persist the reverted context-menu setting"
                );
            }
            self.notifications
                .error(t!("settings.shell_integration_failed", error = error).to_string());
        }
    }
}
