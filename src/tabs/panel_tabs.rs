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

    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() || index == self.active_tab {
            return false;
        }
        self.active_tab = index;
        true
    }

    pub fn close(&mut self, index: usize) -> Option<bool> {
        let (active_tab, was_active) =
            active_index_after_close(self.active_tab, index, self.tabs.len())?;
        self.tabs.remove(index);
        self.active_tab = active_tab;
        Some(was_active)
    }

    pub fn remove_deleted_paths_from_histories(&mut self, deleted_paths: &[PathBuf]) {
        for tab in &mut self.tabs {
            if tab.snapshot.navigation.remove_paths_under(deleted_paths) {
                tab.snapshot.redirect_to_history_current();
            }
        }
    }
}

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
                    snapshot: left,
                }],
                active_tab: 0,
            },
            right: PanelTabSet {
                tabs: vec![PanelTab {
                    id: 1,
                    snapshot: right,
                }],
                active_tab: 0,
            },
            next_id: 2,
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
        if !self.can_add_tab() {
            return None;
        }

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let tabs = self.panel_mut(panel);
        let insert_at = tabs.active_tab + 1;
        tabs.tabs.insert(insert_at, PanelTab { id, snapshot });
        tabs.active_tab = insert_at;
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
