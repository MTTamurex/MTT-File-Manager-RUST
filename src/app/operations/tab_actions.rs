//! Application-level tab actions keep live state, media and persistence in sync.
use crate::app::{dual_panel::ActivePanel, ImageViewerApp};
use crate::tabs::actions::{close_candidates, detach_snapshot, TabAction};

impl ImageViewerApp {
    pub(crate) fn close_main_tab(&mut self, index: usize) {
        let Some(tab) = self.tab_manager.tabs.get(index) else {
            return;
        };
        if tab.pinned {
            return;
        }
        if self.media_preview_owner_tab_id == Some(tab.id) {
            self.destroy_media_preview();
        }
        self.sync_to_tab();
        let closing_active = index == self.tab_manager.active_tab;
        if self.tab_manager.close_tab(index) {
            self.save_preferences();
            self.request_window_close(&self.ui_ctx.clone());
            return;
        }
        if closing_active {
            self.sync_from_tab();
        }
        self.update_video_visibility();
        self.save_preferences();
    }

    pub(crate) fn main_tab_action(&mut self, index: usize, action: TabAction) {
        if index >= self.tab_manager.tabs.len() {
            return;
        }
        match action {
            TabAction::PinQuickAccess | TabAction::UnpinQuickAccess => {
                let path = if index == self.tab_manager.active_tab {
                    &self.navigation_state.current_path
                } else {
                    &self.tab_manager.tabs[index].path
                }
                .clone();
                self.apply_tab_quick_access_action(&path, action);
            }
            TabAction::Close => self.close_main_tab(index),
            TabAction::Duplicate => {
                if !self.tab_manager.can_add_tab() {
                    self.notifications.warning(
                        rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_TABS).to_string(),
                    );
                    return;
                }
                self.sync_to_tab();
                if self.tab_manager.duplicate_at(index) {
                    self.sync_from_tab();
                    if self.navigation_state.is_recycle_bin_view {
                        self.setup_recycle_bin_view();
                    }
                    self.update_video_visibility();
                    self.save_preferences();
                }
            }
            TabAction::TogglePin => {
                self.sync_to_tab();
                self.tab_manager.toggle_pin(index);
                self.save_preferences();
            }
            TabAction::CloseLeft | TabAction::CloseRight => {
                let candidates =
                    close_candidates(&self.tab_manager.tabs, index, action, |tab| tab.pinned);
                if candidates.contains(&self.tab_manager.active_tab) {
                    self.sync_to_tab();
                    self.tab_manager.switch_to(index);
                    self.sync_from_tab();
                }
                for index in candidates {
                    self.close_main_tab(index);
                }
            }
        }
    }

    pub(crate) fn panel_tab_action(&mut self, panel: ActivePanel, index: usize, action: TabAction) {
        if !self.dual_panel_enabled {
            return;
        }
        self.sync_to_tab();
        let Some(tabs) = self.tab_manager.active().dual_panel_tabs.as_ref() else {
            return;
        };
        let set = tabs.panel(panel);
        if index >= set.tabs.len() {
            return;
        }
        match action {
            TabAction::PinQuickAccess | TabAction::UnpinQuickAccess => {
                let path = set.tabs[index].snapshot.path.clone();
                self.apply_tab_quick_access_action(&path, action);
            }
            TabAction::Close => {
                self.dual_panel_close_tab(panel, index);
            }
            TabAction::TogglePin => {
                self.tab_manager
                    .active_mut()
                    .dual_panel_tabs
                    .as_mut()
                    .unwrap()
                    .panel_mut(panel)
                    .toggle_pin(index);
                self.save_preferences();
            }
            TabAction::Duplicate => {
                if !tabs.can_add_tab() {
                    self.notifications.warning(
                        rust_i18n::t!("tabs.max_reached", max = crate::tabs::MAX_PANEL_TABS)
                            .to_string(),
                    );
                    return;
                }
                let mut snapshot = set.tabs[index].snapshot.clone();
                detach_snapshot(&mut snapshot);
                let boundary = set.tabs.iter().take_while(|tab| tab.pinned).count();
                let insert = (index + 1).max(boundary);
                let previous_active = set.active_tab;
                let new_index = self
                    .tab_manager
                    .active_mut()
                    .dual_panel_tabs
                    .as_mut()
                    .unwrap()
                    .add_tab(panel, snapshot)
                    .unwrap();
                // add_tab selects the new tab in storage; restore the previous
                // index until select_tab can apply its live snapshot properly.
                let set = self
                    .tab_manager
                    .active_mut()
                    .dual_panel_tabs
                    .as_mut()
                    .unwrap()
                    .panel_mut(panel);
                set.active_tab = previous_active;
                set.move_tab_to_gap(new_index, insert);
                self.dual_panel_select_tab(panel, insert);
            }
            TabAction::CloseLeft | TabAction::CloseRight => {
                let candidates = close_candidates(&set.tabs, index, action, |tab| tab.pinned);
                if candidates.contains(&set.active_tab) {
                    self.dual_panel_select_tab(panel, index);
                }
                for index in candidates {
                    self.dual_panel_close_tab(panel, index);
                }
            }
        }
    }

    fn apply_tab_quick_access_action(&mut self, path: &str, action: TabAction) {
        use crate::tabs::quick_access::{pinned_folder_for_path, tab_quick_access_action};
        let destination = self.resolve_network_location_path(std::path::Path::new(path));
        let path = destination.to_string_lossy();
        let path = path.as_ref();
        if tab_quick_access_action(path, &self.pinned_folders).is_none() {
            return;
        }
        match action {
            TabAction::PinQuickAccess => {
                if pinned_folder_for_path(path, &self.pinned_folders).is_none() {
                    self.pin_folder(path);
                }
            }
            TabAction::UnpinQuickAccess => {
                if let Some(folder) = pinned_folder_for_path(path, &self.pinned_folders) {
                    let saved_path = folder.path.clone();
                    self.unpin_folder(&saved_path);
                }
            }
            _ => return,
        }
        self.ui_ctx.request_repaint();
    }
}
