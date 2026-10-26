use crate::app::dual_panel::{ActivePanel, PanelSnapshot};
use std::path::PathBuf;

/// Maximum number of inner tabs shared by both panes of one outer workspace.
pub const MAX_PANEL_TABS: usize = 20;

fn can_add_panel_tab_count(left: usize, right: usize) -> bool {
    left.saturating_add(right) < MAX_PANEL_TABS
}

fn active_index_after_close(
    active_tab: usize,
    index: usize,
    tab_count: usize,
) -> Option<(usize, bool)> {
    if tab_count <= 1 || index >= tab_count {
        return None;
    }

    let was_active = index == active_tab;
    let remaining_count = tab_count - 1;
    let active_tab = if active_tab >= remaining_count {
        remaining_count - 1
    } else if active_tab > index {
        active_tab - 1
    } else {
        active_tab
    };
    Some((active_tab, was_active))
}

#[derive(Clone)]
pub struct PanelTab {
    pub id: usize,
    pub pinned: bool,
    pub snapshot: PanelSnapshot,
}

#[derive(Clone, Default)]
pub struct PanelTabSet {
    pub tabs: Vec<PanelTab>,
    pub active_tab: usize,
}

impl PanelTabSet {
    pub fn active(&self) -> Option<&PanelTab> {
        self.tabs.get(self.active_tab)
    }

    pub fn active_mut(&mut self) -> Option<&mut PanelTab> {
        self.tabs.get_mut(self.active_tab)
    }

    /// A dormant pinned tab must not inherit navigation from single-panel mode.
    pub(crate) fn sync_active_snapshot(
        &mut self,
        snapshot: PanelSnapshot,
        dual_panel_enabled: bool,
    ) {
        if let Some(tab) = self
            .active_mut()
            .filter(|tab| dual_panel_enabled || !tab.pinned)
        {
            tab.snapshot = snapshot;
        }
    }

    pub(crate) fn pinned_snapshot_for_resume(&self) -> Option<PanelSnapshot> {
        self.active()
            .filter(|tab| tab.pinned)
            .map(|tab| tab.snapshot.clone())
    }

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() || index == self.active_tab {
            return false;
        }
        self.active_tab = index;
        true
    }

    pub fn close(&mut self, index: usize) -> Option<bool> {
        if self.tabs.get(index)?.pinned {
            return None;
        }
        let (active_tab, was_active) =
            active_index_after_close(self.active_tab, index, self.tabs.len())?;
        self.tabs.remove(index);
        self.active_tab = active_tab;
        Some(was_active)
    }

    pub fn move_tab_to_gap(&mut self, from: usize, gap: usize) -> bool {
        if !super::actions::valid_move(&self.tabs, from, gap, |tab| tab.pinned) {
            return false;
        }
        super::reorder::move_to_gap(&mut self.tabs, &mut self.active_tab, from, gap)
    }

    pub fn remove_deleted_paths_from_histories(&mut self, deleted_paths: &[PathBuf]) {
        for tab in &mut self.tabs {
            if tab.snapshot.navigation.remove_paths_under(deleted_paths) {
                tab.snapshot.redirect_to_history_current();
            }
        }
    }
}

#[cfg(test)]
mod snapshot_sync_tests;

#[derive(Clone, Default)]
pub struct DualPanelTabs {
    pub left: PanelTabSet,
    pub right: PanelTabSet,
    next_id: usize,
}

impl DualPanelTabs {
    pub fn new(left: PanelSnapshot, right: PanelSnapshot) -> Self {
        Self {
            left: PanelTabSet {
                tabs: vec![PanelTab {
                    id: 0,
                    pinned: false,
                    snapshot: left,
                }],
                active_tab: 0,
            },
            right: PanelTabSet {
                tabs: vec![PanelTab {
                    id: 1,
                    pinned: false,
                    snapshot: right,
                }],
                active_tab: 0,
            },
            next_id: 2,
        }
    }

    pub(crate) fn from_panel_tab_sets(left: PanelTabSet, right: PanelTabSet) -> Self {
        let next_id = left
            .tabs
            .iter()
            .chain(&right.tabs)
            .map(|tab| tab.id)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        Self {
            left,
            right,
            next_id,
        }
    }

    pub fn panel(&self, panel: ActivePanel) -> &PanelTabSet {
        match panel {
            ActivePanel::Left => &self.left,
            ActivePanel::Right => &self.right,
        }
    }

    pub fn panel_mut(&mut self, panel: ActivePanel) -> &mut PanelTabSet {
        match panel {
            ActivePanel::Left => &mut self.left,
            ActivePanel::Right => &mut self.right,
        }
    }

    pub fn count(&self) -> usize {
        self.left.tabs.len() + self.right.tabs.len()
    }

    pub fn can_add_tab(&self) -> bool {
        can_add_panel_tab_count(self.left.tabs.len(), self.right.tabs.len())
    }

    pub fn add_tab(&mut self, panel: ActivePanel, snapshot: PanelSnapshot) -> Option<usize> {
        let index = self.add_background_tab(panel, snapshot)?;
        self.panel_mut(panel).active_tab = index;
        Some(index)
    }

    pub(crate) fn add_background_tab(
        &mut self,
        panel: ActivePanel,
        snapshot: PanelSnapshot,
    ) -> Option<usize> {
        if !self.can_add_tab() {
            return None;
        }

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let tabs = self.panel_mut(panel);
        let insert_at = tabs.tabs.len();
        tabs.tabs.push(PanelTab {
            id,
            pinned: false,
            snapshot,
        });
        Some(insert_at)
    }

    pub fn remove_deleted_paths_from_histories(&mut self, deleted_paths: &[PathBuf]) {
        self.left.remove_deleted_paths_from_histories(deleted_paths);
        self.right
            .remove_deleted_paths_from_histories(deleted_paths);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_tab_limit_is_shared_between_both_sides() {
        assert!(can_add_panel_tab_count(10, 9));
        assert!(!can_add_panel_tab_count(10, 10));
    }

    #[test]
    fn closing_panel_tab_keeps_one_and_tracks_active_index() {
        assert_eq!(active_index_after_close(2, 1, 3), Some((1, false)));
        assert_eq!(active_index_after_close(1, 1, 3), Some((1, true)));
        assert_eq!(active_index_after_close(1, 0, 2), Some((0, false)));
        assert_eq!(active_index_after_close(0, 0, 1), None);
    }
}
