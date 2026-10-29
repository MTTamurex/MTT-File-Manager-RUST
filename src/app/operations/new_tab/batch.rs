use crate::app::{dual_panel::ActivePanel, ImageViewerApp};
use std::path::Path;

impl ImageViewerApp {
    pub fn dual_panel_add_tab(&mut self, panel: ActivePanel) -> bool {
        if !self.dual_panel_enabled || self.tab_navigation_blocked() {
            return false;
        }
        let current_path = if panel == self.dual_panel_active {
            self.navigation_state.current_path.as_str()
        } else {
            self.dual_panel_inactive_path()
                .unwrap_or(&self.navigation_state.current_path)
        };
        let path = self
            .tab_preferences
            .location
            .resolve(current_path, &self.tab_preferences.custom_path);
        let ctrl = self.ui_ctx.input(|input| input.modifiers.ctrl);
        self.open_locations_in_new_tabs(panel, &[path], ctrl)
    }

    /// Paths and modifiers must be captured before changing the source tab.
    pub(crate) fn open_locations_in_new_tabs(
        &mut self,
        panel: ActivePanel,
        paths: &[String],
        ctrl: bool,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        if self.dual_panel_enabled {
            self.open_panel_tabs(panel, paths, ctrl)
        } else {
            self.open_main_tabs(paths, ctrl)
        }
    }

    pub(super) fn open_main_tabs(&mut self, paths: &[String], ctrl: bool) -> bool {
        self.sync_to_tab();
        let view_mode = self.new_tab_view_mode();
        let mut first = None;
        for path in paths {
            if !self.tab_manager.can_add_tab() {
                self.notifications.warning(
                    rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
                );
                break;
            }
            let destination = self.resolve_network_location_path(Path::new(path));
            let modified = self.resolve_destination_folder_modified_hint(&destination);
            let created = self.resolve_destination_folder_created_hint(&destination);
            let Some(index) = self
                .tab_manager
                .new_background_tab_at(&destination.to_string_lossy())
            else {
                break;
            };
            let tab = &mut self.tab_manager.tabs[index];
            tab.view_mode = view_mode;
            tab.sort_mode = self.sort_mode;
            tab.sort_descending = self.sort_descending;
            tab.folders_position = self.folders_position;
            tab.current_folder_modified_hint = modified;
            tab.current_folder_created_hint = created;
            first.get_or_insert(index);
        }
        if let Some(index) = first {
            if self.tab_preferences.should_activate_new_tab(ctrl) {
                self.tab_manager.switch_to(index);
                self.sync_from_tab();
                self.sync_to_tab();
                self.update_video_visibility();
            }
            self.save_preferences();
        }
        first.is_some()
    }

    fn open_panel_tabs(&mut self, panel: ActivePanel, paths: &[String], ctrl: bool) -> bool {
        self.sync_to_tab();
        let mut first = None;
        for path in paths {
            if !self
                .tab_manager
                .active()
                .dual_panel_tabs
                .as_ref()
                .is_some_and(|tabs| tabs.can_add_tab())
            {
                self.notifications.warning(
                    rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_PANEL_TABS)
                        .to_string(),
                );
                break;
            }
            let destination = self.resolve_network_location_path(Path::new(path));
            let destination = destination.to_string_lossy();
            // Capture settings from the physical origin without focusing it.
            let snapshot = if panel == self.dual_panel_active {
                self.new_panel_tab_snapshot(&destination)
            } else {
                let Some(snapshot) =
                    self.with_inactive_panel(|app| app.new_panel_tab_snapshot(&destination))
                else {
                    break;
                };
                snapshot
            };
            let index = self
                .tab_manager
                .active_mut()
                .dual_panel_tabs
                .as_mut()
                .and_then(|tabs| tabs.add_background_tab(panel, snapshot));
            if let Some(index) = index {
                first.get_or_insert(index);
            }
        }
        if let Some(index) = first {
            if self.tab_preferences.should_activate_new_tab(ctrl) {
                self.dual_panel_select_tab(panel, index);
            } else {
                self.save_preferences();
            }
        }
        first.is_some()
    }
}
