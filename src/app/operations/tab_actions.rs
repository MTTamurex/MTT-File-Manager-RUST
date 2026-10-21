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
                let candidates = close_candidates(
                    &self.tab_manager.tabs,
                    index,
                    self.tab_manager.active_tab,
                    action,
                    |tab| tab.pinned,
                );
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
                let candidates =
                    close_candidates(&set.tabs, index, set.active_tab, action, |tab| tab.pinned);
                for index in candidates {
                    self.dual_panel_close_tab(panel, index);
                }
            }
        }
    }
}
