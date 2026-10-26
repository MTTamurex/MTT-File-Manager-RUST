//! Lightweight transfer records; listings and GPU resources stay process-local.
use super::session::SessionPanelTabSet;
use super::{TabManager, TabManagerSession, TabState};
use crate::app::dual_panel::PanelSnapshot;
use crate::application::navigation::NavigationHistory;
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod validation_tests;

impl TabManager {
    pub(crate) fn can_detach_tab(&self, index: usize) -> bool {
        self.tabs.len() > 1 && index < self.tabs.len()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TabTransfer {
    pub(crate) session: TabManagerSession,
    browser: BrowserState,
    panels: Option<(Vec<BrowserState>, Vec<BrowserState>)>,
}

#[derive(Clone, Serialize, Deserialize)]
struct BrowserState {
    history: Vec<String>,
    history_index: usize,
    search: String,
    scroll_x: f32,
    scroll_y: f32,
}

impl BrowserState {
    fn capture(navigation: &NavigationHistory, search: &str, x: f32, y: f32) -> Self {
        Self {
            history: navigation.paths.iter().cloned().collect(),
            history_index: navigation.current_index,
            search: search.to_owned(),
            scroll_x: x,
            scroll_y: y,
        }
    }

    fn from_panel(snapshot: &PanelSnapshot) -> Self {
        Self::capture(
            &snapshot.navigation,
            &snapshot.search_query,
            snapshot.scroll_offset_x,
            snapshot.scroll_offset_y,
        )
    }

    fn navigation(&self, path: &str) -> NavigationHistory {
        let mut navigation = NavigationHistory::new(path.to_owned());
        if self.history.len() <= 500
            && self
                .history
                .get(self.history_index)
                .is_some_and(|current| current == path)
        {
            navigation.paths = self.history.iter().cloned().collect();
            navigation.current_index = self.history_index;
        }
        navigation
    }

    fn valid_for(&self, path: &str) -> bool {
        !self.history.is_empty()
            && self.history.len() <= 500
            && self
                .history
                .get(self.history_index)
                .is_some_and(|current| current == path)
            && self.scroll_x.is_finite()
            && self.scroll_y.is_finite()
    }

    fn restore_panel(&self, snapshot: &mut PanelSnapshot) {
        snapshot.navigation = self.navigation(&snapshot.path);
        snapshot.search_query = self.search.clone();
        snapshot.scroll_offset_x = self.scroll_x;
        snapshot.scroll_offset_y = self.scroll_y;
    }
}

impl TabTransfer {
    pub(crate) fn from_tab(tab: &TabState) -> Self {
        Self {
            session: TabManagerSession::for_transferred_tab(tab),
            browser: BrowserState::capture(
                &tab.navigation,
                &tab.search_query,
                tab.scroll_offset_x,
                tab.scroll_offset_y,
            ),
            panels: tab.dual_panel_tabs.as_ref().map(|tabs| {
                (
                    tabs.left
                        .tabs
                        .iter()
                        .map(|tab| BrowserState::from_panel(&tab.snapshot))
                        .collect(),
                    tabs.right
                        .tabs
                        .iter()
                        .map(|tab| BrowserState::from_panel(&tab.snapshot))
                        .collect(),
                )
            }),
        }
    }

    pub(crate) fn from_panel(snapshot: &PanelSnapshot, pinned: bool, workspace: &TabState) -> Self {
        Self {
            session: TabManagerSession::for_transferred_panel(snapshot, pinned, workspace),
            browser: BrowserState::from_panel(snapshot),
            panels: None,
        }
    }

    pub(crate) fn from_json(value: &str) -> Option<Self> {
        if value.len() > 1024 * 1024 {
            return None;
        }
        let mut transfer: Self = serde_json::from_str(value).ok()?;
        let validated = TabManagerSession::from_json(&transfer.session.to_json().ok()?)?;
        // Session restoration deliberately repairs damaged persisted data. A
        // move must instead fail rather than acknowledge a lossy reconstruction.
        if validated != transfer.session || validated.tabs.len() != 1 {
            return None;
        }
        transfer.session = validated;
        let tab = &transfer.session.tabs[0];
        if !transfer.browser.valid_for(&tab.path) {
            return None;
        }
        match (&tab.dual_panel_tabs, &transfer.panels) {
            (Some(saved), Some((left, right))) => {
                for (set, states) in [(&saved.left, left), (&saved.right, right)] {
                    if set.tabs.len() != states.len()
                        || !set
                            .tabs
                            .iter()
                            .zip(states)
                            .all(|(tab, state)| state.valid_for(&tab.path))
                    {
                        return None;
                    }
                }
            }
            (None, None) if !tab.dual_panel_enabled => {}
            _ => return None,
        }
        Some(transfer)
    }

    pub(crate) fn locations_match(&self, manager: &TabManager) -> bool {
        let actual = TabManagerSession::from_tab_manager(manager);
        if actual.tabs.len() != 1 {
            return false;
        }
        let expected = &self.session.tabs[0];
        let actual = &actual.tabs[0];
        expected.path == actual.path
            && expected.pinned == actual.pinned
            && expected.dual_panel_enabled == actual.dual_panel_enabled
            && expected.dual_panel_active == actual.dual_panel_active
            && match (&expected.dual_panel_tabs, &actual.dual_panel_tabs) {
                (None, None) => true,
                (Some(expected), Some(actual)) => {
                    panel_locations_match(&expected.left, &actual.left)
                        && panel_locations_match(&expected.right, &actual.right)
                }
                _ => false,
            }
    }

    pub(crate) fn restore(&self, app: &mut crate::app::ImageViewerApp) {
        let active = app.tab_manager.active_mut();
        active.navigation = self.browser.navigation(&active.path);
        active.search_query = self.browser.search.clone();
        active.scroll_offset_x = self.browser.scroll_x;
        active.scroll_offset_y = self.browser.scroll_y;
        if let (Some(tabs), Some((left, right))) = (&mut active.dual_panel_tabs, &self.panels) {
            for (tab, state) in tabs.left.tabs.iter_mut().zip(left) {
                state.restore_panel(&mut tab.snapshot);
            }
            for (tab, state) in tabs.right.tabs.iter_mut().zip(right) {
                state.restore_panel(&mut tab.snapshot);
            }
            if let (Some(inactive), Some(tab)) = (
                &mut app.dual_panel_inactive_state,
                tabs.panel(app.dual_panel_active.other()).active(),
            ) {
                inactive.navigation = tab.snapshot.navigation.clone();
                inactive.search_query = tab.snapshot.search_query.clone();
                inactive.scroll_offset_x = tab.snapshot.scroll_offset_x;
                inactive.scroll_offset_y = tab.snapshot.scroll_offset_y;
            }
        }
        app.navigation_state.navigation = active.navigation.clone();
        app.search_query = active.search_query.clone();
        app.scroll_offset_x = active.scroll_offset_x;
        app.scroll_offset_y = active.scroll_offset_y;
        // Synchronous views (e.g. This PC) already have their listing, so merely
        // assigning the query would show unfiltered results until another edit.
        if !app.search_query.is_empty() {
            app.filter_items();
        }
        if app.dual_panel_enabled {
            app.with_inactive_panel(|app| {
                if !app.search_query.is_empty() {
                    app.filter_items();
                }
            });
        }
    }
}

fn panel_locations_match(expected: &SessionPanelTabSet, actual: &SessionPanelTabSet) -> bool {
    expected.active_tab == actual.active_tab
        && expected.tabs.len() == actual.tabs.len()
        && expected
            .tabs
            .iter()
            .zip(&actual.tabs)
            .all(|(expected, actual)| {
                expected.path == actual.path && expected.pinned == actual.pinned
            })
}

impl TabManagerSession {
    pub(crate) fn for_transferred_tab(tab: &TabState) -> Self {
        let mut manager = TabManager::new();
        manager.tabs = vec![tab.clone()];
        Self::from_tab_manager(&manager)
    }

    pub(crate) fn for_transferred_panel(
        snapshot: &PanelSnapshot,
        pinned: bool,
        workspace: &TabState,
    ) -> Self {
        let mut tab = TabState::new_at_path(0, &snapshot.path);
        tab.pinned = pinned;
        tab.show_left_sidebar = workspace.show_left_sidebar;
        tab.show_preview_panel = workspace.show_preview_panel;
        tab.collapse_tags = workspace.collapse_tags;
        tab.view_mode = snapshot.view_mode;
        tab.sort_mode = snapshot.sort_mode;
        tab.sort_descending = snapshot.sort_descending;
        tab.folders_position = snapshot.folders_position;
        tab.group_mode = snapshot.group_mode;
        tab.group_descending = snapshot.group_descending;
        tab.active_tag_filter = snapshot.active_tag_filter;
        Self::for_transferred_tab(&tab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::{GroupMode, ViewMode};

    #[test]
    fn last_workspace_cannot_be_detached() {
        let mut manager = TabManager::new();
        assert!(!manager.can_detach_tab(0));
        manager.new_tab();
        assert!(manager.can_detach_tab(0));
        assert!(manager.can_detach_tab(1));
        assert!(!manager.can_detach_tab(2));
        manager.close_tab(1);
        assert!(!manager.can_detach_tab(0));
    }

    #[test]
    fn transfer_contains_only_the_chosen_workspace_and_keeps_browser_settings() {
        let mut manager = TabManager::new();
        manager.new_tab_at("C:\\");
        let tab = manager.active_mut();
        tab.navigate_to("D:\\");
        tab.pinned = true;
        tab.view_mode = ViewMode::List;
        tab.group_mode = GroupMode::Type;
        tab.search_query = "file".to_owned();
        tab.scroll_offset_y = 100.0;
        let serialized = serde_json::to_string(&TabTransfer::from_tab(tab)).unwrap();
        let transfer = TabTransfer::from_json(&serialized).unwrap();
        let restored = transfer.session.clone().into_tab_manager();
        assert_eq!(restored.tabs.len(), 1);
        assert_eq!(restored.active().path, "D:\\");
        assert!(restored.active().pinned);
        assert_eq!(restored.active().view_mode, ViewMode::List);
        assert_eq!(transfer.browser.navigation("D:\\").paths.len(), 2);
        assert_eq!(transfer.browser.search, "file");
        assert_eq!(transfer.browser.scroll_y, 100.0);
    }

    #[test]
    fn malformed_transfer_does_not_restore_an_unrelated_session() {
        assert!(TabTransfer::from_json("{}").is_none());
        assert!(TabTransfer::from_json(&" ".repeat(1024 * 1024 + 1)).is_none());
    }

    #[test]
    fn panel_transfer_keeps_workspace_visibility_and_panel_browser_state() {
        let snapshot = crate::tabs::panel_tabs::snapshot_sync_tests::snapshot("C:\\");
        let mut workspace = TabState::new_at_computer(0);
        workspace.show_left_sidebar = false;
        workspace.show_preview_panel = false;
        workspace.collapse_tags = true;
        let transfer = TabTransfer::from_panel(&snapshot, true, &workspace);
        let transfer = TabTransfer::from_json(&serde_json::to_string(&transfer).unwrap()).unwrap();
        let restored = transfer.session.clone().into_tab_manager();
        let tab = restored.active();
        assert!(!tab.show_left_sidebar);
        assert!(!tab.show_preview_panel);
        assert!(tab.collapse_tags);
        assert!(tab.pinned);
        assert_eq!(tab.view_mode, snapshot.view_mode);
        assert!(!tab.dual_panel_enabled);
        assert_eq!(transfer.browser.search, snapshot.search_query);
    }
}
