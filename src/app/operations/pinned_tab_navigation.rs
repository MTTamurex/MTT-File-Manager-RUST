use crate::app::{dual_panel::ActivePanel, ImageViewerApp};

pub(crate) struct PendingNavigation {
    workspace: usize,
    inner_tab: Option<usize>,
    source_path: String,
    panel: ActivePanel,
    destination: String,
}

impl ImageViewerApp {
    pub(crate) fn active_path_is_pinned(&self) -> bool {
        let panel = if self.in_inactive_panel_context {
            self.dual_panel_active.other()
        } else {
            self.dual_panel_active
        };
        self.panel_path_is_pinned(panel)
    }

    pub(super) fn panel_path_is_pinned(&self, panel: ActivePanel) -> bool {
        if self.dual_panel_enabled {
            self.tab_manager
                .active()
                .dual_panel_tabs
                .as_ref()
                .and_then(|tabs| tabs.panel(panel).active())
                .is_some_and(|tab| tab.pinned)
        } else {
            self.tab_manager.active().pinned
        }
    }

    /// Returns true even at the tab limit: the locked source must stay untouched.
    pub(crate) fn redirect_pinned_navigation(&mut self, path: &str) -> bool {
        if path == self.navigation_state.current_path || !self.active_path_is_pinned() {
            return false;
        }
        self.navigation_state.path_input = self.navigation_state.current_path.clone();
        if self.in_inactive_panel_context
            || crate::ui::new_tab_request::is_rendering_panel(&self.ui_ctx)
        {
            // Applying a tab while rendering could corrupt either pane's live
            // state. Consume this only after both panes finish rendering.
            let panel = if self.in_inactive_panel_context {
                self.dual_panel_active.other()
            } else {
                self.dual_panel_active
            };
            let workspace = self.tab_manager.active();
            self.pending_pinned_navigation = Some(PendingNavigation {
                workspace: workspace.id,
                inner_tab: workspace
                    .dual_panel_tabs
                    .as_ref()
                    .and_then(|tabs| tabs.panel(panel).active())
                    .map(|tab| tab.id),
                source_path: self.navigation_state.current_path.clone(),
                panel,
                destination: path.to_string(),
            });
        } else {
            self.open_pinned_destination(self.dual_panel_active, path);
        }
        true
    }

    pub(crate) fn process_pending_pinned_navigation(&mut self) {
        if self.in_inactive_panel_context || self.tab_navigation_blocked() {
            return;
        }
        if let Some(request) = self.pending_pinned_navigation.take() {
            let workspace = self.tab_manager.active();
            let inner = workspace
                .dual_panel_tabs
                .as_ref()
                .and_then(|tabs| tabs.panel(request.panel).active())
                .map(|tab| tab.id);
            let path = if self.dual_panel_enabled && request.panel != self.dual_panel_active {
                self.dual_panel_inactive_path()
            } else {
                Some(self.navigation_state.current_path.as_str())
            };
            if workspace.id == request.workspace
                && inner == request.inner_tab
                && path == Some(request.source_path.as_str())
            {
                self.open_pinned_destination(request.panel, &request.destination);
            }
        }
    }

    pub(super) fn open_pinned_destination(&mut self, panel: ActivePanel, path: &str) {
        let source = if self.dual_panel_enabled {
            self.tab_manager
                .active()
                .dual_panel_tabs
                .as_ref()
                .map(|tabs| {
                    let set = tabs.panel(panel);
                    (set.active_tab, set.tabs.len())
                })
        } else {
            Some((self.tab_manager.active_tab, self.tab_manager.tabs.len()))
        };
        let Some((source, count)) = source else {
            return;
        };
        self.open_location_in_new_tab(panel, path);
        if self.dual_panel_enabled {
            if let Some(tabs) = self.tab_manager.active_mut().dual_panel_tabs.as_mut() {
                let set = tabs.panel_mut(panel);
                if set.tabs.len() > count {
                    let boundary = set.tabs.iter().take_while(|tab| tab.pinned).count();
                    set.move_tab_to_gap(set.active_tab, (source + 1).max(boundary));
                }
            }
        } else if self.tab_manager.tabs.len() > count {
            let boundary = self
                .tab_manager
                .tabs
                .iter()
                .take_while(|tab| tab.pinned)
                .count();
            self.tab_manager
                .move_tab_to_gap(self.tab_manager.active_tab, (source + 1).max(boundary));
        }
        self.save_preferences();
    }
}
