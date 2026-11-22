//! Persisted workspace state for saved tab groups.

use super::{DualPanelTabs, PanelTabSet, TabManager, TabState};
use crate::app::dual_panel::ActivePanel;
use crate::application::navigation::NavigationHistory;
use crate::domain::folder_group::{
    SavedGroupDualPanel, SavedGroupPanel, SavedGroupPanelTab, SavedGroupPanelTabs, SavedGroupTab,
    SavedGroupTabs,
};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};

impl TabManager {
    /// Captures every outer tab and, for dual-panel workspaces, both inner tab
    /// sets with their active indices. Call after `sync_to_tab` so the active
    /// panel's live state is reflected in `dual_panel_tabs`.
    pub(crate) fn saved_group_tabs(&self) -> SavedGroupTabs {
        let mut saved = SavedGroupTabs {
            active_tab: self.active_tab,
            tabs: self
                .tabs
                .iter()
                .map(|tab| SavedGroupTab {
                    path: tab.path.clone(),
                    view_mode: Some(tab.view_mode.preference_value().to_owned()),
                    dual_panel: if tab.dual_panel_enabled {
                        tab.dual_panel_tabs
                            .as_ref()
                            .map(|panels| SavedGroupDualPanel {
                                active: saved_panel(tab.dual_panel_active),
                                split_ratio: tab.dual_panel_split_ratio,
                                left: SavedGroupPanelTabs::from_tab_set(&panels.left),
                                right: SavedGroupPanelTabs::from_tab_set(&panels.right),
                            })
                    } else {
                        None
                    },
                })
                .collect(),
        };
        saved.sanitize();
        saved
    }
}

impl SavedGroupPanelTabs {
    fn from_tab_set(set: &PanelTabSet) -> Self {
        Self {
            active: set.active_tab,
            tabs: set
                .tabs
                .iter()
                .map(|tab| SavedGroupPanelTab {
                    path: tab.snapshot.path.clone(),
                    view_mode: Some(tab.snapshot.view_mode.preference_value().to_owned()),
                    pinned: tab.pinned,
                })
                .collect(),
        }
    }
}

impl TabState {
    pub(crate) fn restore_saved_group_dual_panel(
        &mut self,
        saved: &SavedGroupDualPanel,
        left: PanelTabSet,
        right: PanelTabSet,
    ) {
        let active = match saved.active {
            SavedGroupPanel::Left => ActivePanel::Left,
            SavedGroupPanel::Right => ActivePanel::Right,
        };
        let active_path = match active {
            ActivePanel::Left => left.active(),
            ActivePanel::Right => right.active(),
        }
        .map(|tab| tab.snapshot.path.clone());

        if let Some(path) = active_path {
            self.path = path.clone();
            self.path_input = path.clone();
            self.is_computer_view = path == COMPUTER_VIEW_ID;
            self.is_recycle_bin_view = path == RECYCLE_BIN_VIEW_ID;
            self.navigation = NavigationHistory::new(path.clone());
            self.title = super::tab_title_for_path(&path);
        }
        if let Some(active_tab) = match active {
            ActivePanel::Left => left.active(),
            ActivePanel::Right => right.active(),
        } {
            self.view_mode = active_tab.snapshot.view_mode;
            self.sort_mode = active_tab.snapshot.sort_mode;
            self.sort_descending = active_tab.snapshot.sort_descending;
            self.folders_position = active_tab.snapshot.folders_position;
            self.group_mode = active_tab.snapshot.group_mode;
            self.group_descending = active_tab.snapshot.group_descending;
        }

        self.dual_panel_enabled = true;
        self.dual_panel_active = active;
        self.dual_panel_split_ratio = if saved.split_ratio.is_finite() {
            saved.split_ratio.clamp(0.0, 1.0)
        } else {
            0.5
        };
        self.dual_panel_tabs = Some(DualPanelTabs::from_panel_tab_sets(left, right));
    }
}

fn saved_panel(panel: ActivePanel) -> SavedGroupPanel {
    match panel {
        ActivePanel::Left => SavedGroupPanel::Left,
        ActivePanel::Right => SavedGroupPanel::Right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs::panel_tabs::snapshot_sync_tests::snapshot;
    use crate::tabs::{DualPanelTabs, PanelTab};

    #[test]
    fn saved_group_tabs_capture_dual_panel_inner_tabs_and_active_indices() {
        let mut manager = TabManager::new_at_path(r"E:\Media\Folder");
        manager.active_mut().view_mode = crate::domain::file_entry::ViewMode::Miller;
        assert!(manager.new_tab_at(r"C:\Users\user\Documents\Project"));
        manager.switch_to(1);
        manager.active_mut().dual_panel_enabled = true;
        manager.active_mut().dual_panel_active = ActivePanel::Right;
        manager.active_mut().dual_panel_split_ratio = 0.65;
        manager.active_mut().path = r"D:\Videos\FolderB".to_owned();
        manager.active_mut().view_mode = crate::domain::file_entry::ViewMode::ColumnList;

        let mut panels = DualPanelTabs::new(
            {
                let mut snapshot = snapshot(r"C:\Users\user\Documents\Project\FolderA");
                snapshot.view_mode = crate::domain::file_entry::ViewMode::List;
                snapshot
            },
            {
                let mut snapshot = snapshot(r"D:\Videos\FolderA");
                snapshot.view_mode = crate::domain::file_entry::ViewMode::Grid;
                snapshot
            },
        );
        panels
            .add_background_tab(ActivePanel::Left, {
                let mut snapshot = snapshot(r"C:\Users\user\Documents\Project\FolderB");
                snapshot.view_mode = crate::domain::file_entry::ViewMode::Miller;
                snapshot
            })
            .unwrap();
        panels.left.select(1);
        panels
            .add_background_tab(ActivePanel::Right, {
                let mut snapshot = snapshot(r"D:\Videos\FolderB");
                snapshot.view_mode = crate::domain::file_entry::ViewMode::List;
                snapshot
            })
            .unwrap();
        panels.right.select(1);
        panels.left.tabs[0].pinned = true;
        manager.active_mut().dual_panel_tabs = Some(panels);

        let saved = manager.saved_group_tabs();

        assert_eq!(saved.active_tab, 1);
        assert_eq!(saved.tabs.len(), 2);
        assert!(saved.tabs[0].dual_panel.is_none());
        assert_eq!(saved.tabs[0].view_mode.as_deref(), Some("miller"));
        assert_eq!(saved.tabs[1].path, r"D:\Videos\FolderB");
        assert_eq!(
            saved.tabs[1].view_mode.as_deref().unwrap(),
            crate::domain::file_entry::ViewMode::ColumnList.preference_value()
        );

        let dual = saved.tabs[1].dual_panel.as_ref().unwrap();
        assert_eq!(dual.active, SavedGroupPanel::Right);
        assert!((dual.split_ratio - 0.65).abs() < f32::EPSILON);
        assert_eq!(dual.left.active, 1);
        assert_eq!(
            dual.left.tabs[0].path,
            r"C:\Users\user\Documents\Project\FolderA"
        );
        assert!(dual.left.tabs[0].pinned);
        assert_eq!(
            dual.left.tabs[1].path,
            r"C:\Users\user\Documents\Project\FolderB"
        );
        assert_eq!(dual.left.tabs[1].view_mode.as_deref(), Some("miller"));
        assert_eq!(dual.right.active, 1);
        assert_eq!(dual.right.tabs[1].path, r"D:\Videos\FolderB");
        assert_eq!(dual.right.tabs[0].view_mode.as_deref(), Some("grid"));
        assert_eq!(dual.right.tabs[1].view_mode.as_deref(), Some("list"));
    }

    #[test]
    fn saved_group_tabs_skip_dual_panel_state_when_the_workspace_is_mono() {
        let mut manager = TabManager::new_at_path(r"C:\Users\user\Documents\Project");
        manager.active_mut().dual_panel_enabled = false;
        manager.active_mut().dual_panel_tabs = Some(DualPanelTabs::new(
            snapshot(r"C:\Users\user\Documents\Project\FolderA"),
            snapshot(r"D:\Videos\FolderA"),
        ));

        let saved = manager.saved_group_tabs();

        assert!(saved.tabs[0].dual_panel.is_none());
    }

    #[test]
    fn legacy_saved_tabs_without_view_modes_remain_readable() {
        let saved: SavedGroupTabs = serde_json::from_str(
            r#"{"active_tab":0,"tabs":[{"path":"C:\\Users\\user\\Documents\\Project","dual_panel":null}]}"#,
        )
        .unwrap();

        assert_eq!(saved.tabs[0].view_mode, None);
        assert!(saved.tabs[0].dual_panel.is_none());
    }

    #[test]
    fn legacy_panel_tabs_without_view_modes_remain_readable() {
        let saved: SavedGroupTabs = serde_json::from_str(
            r#"{
                "active_tab": 0,
                "tabs": [{
                    "path": "C:\\",
                    "dual_panel": {
                        "active": "right",
                        "split_ratio": 0.5,
                        "left": {"active": 0, "tabs": [{"path": "C:\\", "pinned": true}]},
                        "right": {"active": 0, "tabs": [{"path": "D:\\"}]}
                    }
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(saved.tabs[0].view_mode, None);
        let dual = saved.tabs[0].dual_panel.as_ref().unwrap();
        assert_eq!(dual.left.tabs[0].view_mode, None);
        assert_eq!(dual.right.tabs[0].view_mode, None);
        assert!(dual.left.tabs[0].pinned);
        assert_eq!(dual.active, SavedGroupPanel::Right);
    }

    #[test]
    fn restoring_saved_dual_panel_reinstates_both_inner_sets_and_active_locations() {
        let saved = SavedGroupDualPanel {
            active: SavedGroupPanel::Right,
            split_ratio: 0.9,
            left: SavedGroupPanelTabs {
                active: 1,
                tabs: vec![
                    SavedGroupPanelTab {
                        path: r"C:\Users\user\Documents\Project\FolderA".to_owned(),
                        view_mode: Some("grid".to_owned()),
                        pinned: false,
                    },
                    SavedGroupPanelTab {
                        path: r"C:\Users\user\Documents\Project\FolderB".to_owned(),
                        view_mode: Some("list".to_owned()),
                        pinned: true,
                    },
                ],
            },
            right: SavedGroupPanelTabs {
                active: 1,
                tabs: vec![
                    SavedGroupPanelTab {
                        path: r"D:\Videos\FolderA".to_owned(),
                        view_mode: Some("column_list".to_owned()),
                        pinned: false,
                    },
                    SavedGroupPanelTab {
                        path: r"D:\Videos\FolderB".to_owned(),
                        view_mode: Some("miller".to_owned()),
                        pinned: false,
                    },
                ],
            },
        };
        let panel_set = |saved: &SavedGroupPanelTabs, offset: usize| PanelTabSet {
            active_tab: saved.active,
            tabs: saved
                .tabs
                .iter()
                .enumerate()
                .map(|(index, tab)| PanelTab {
                    id: offset + index,
                    pinned: tab.pinned,
                    snapshot: {
                        let mut snapshot = snapshot(&tab.path);
                        if let Some(view_mode) = &tab.view_mode {
                            snapshot.view_mode =
                                crate::domain::file_entry::ViewMode::from_preference(view_mode);
                        }
                        snapshot.sort_mode = crate::domain::file_entry::SortMode::Size;
                        snapshot.sort_descending = true;
                        snapshot.folders_position =
                            crate::domain::file_entry::FoldersPosition::Last;
                        snapshot.group_mode = crate::domain::file_entry::GroupMode::Type;
                        snapshot.group_descending = true;
                        snapshot
                    },
                })
                .collect(),
        };
        let mut tab = TabState::new_at_path(3, r"C:\Users\user\Documents\Project");

        tab.restore_saved_group_dual_panel(
            &saved,
            panel_set(&saved.left, 0),
            panel_set(&saved.right, saved.left.tabs.len()),
        );

        assert!(tab.dual_panel_enabled);
        assert_eq!(tab.dual_panel_active, ActivePanel::Right);
        assert_eq!(tab.path, r"D:\Videos\FolderB");
        assert_eq!(tab.path_input, tab.path);
        assert_eq!(tab.view_mode, crate::domain::file_entry::ViewMode::Miller);
        assert_eq!(tab.sort_mode, crate::domain::file_entry::SortMode::Size);
        assert!(tab.sort_descending);
        assert_eq!(
            tab.folders_position,
            crate::domain::file_entry::FoldersPosition::Last
        );
        assert_eq!(tab.group_mode, crate::domain::file_entry::GroupMode::Type);
        assert!(tab.group_descending);
        assert!((tab.dual_panel_split_ratio - 0.9).abs() < f32::EPSILON);
        let panels = tab.dual_panel_tabs.as_ref().unwrap();
        assert_eq!(panels.left.active_tab, 1);
        assert_eq!(panels.left.tabs.len(), 2);
        assert_eq!(
            panels.left.tabs[1].snapshot.path,
            r"C:\Users\user\Documents\Project\FolderB"
        );
        assert!(panels.left.tabs[1].pinned);
        assert_eq!(
            panels.left.tabs[1].snapshot.view_mode,
            crate::domain::file_entry::ViewMode::List
        );
        assert_eq!(panels.right.active_tab, 1);
        assert_eq!(panels.right.tabs.len(), 2);
        assert_eq!(panels.right.tabs[0].snapshot.path, r"D:\Videos\FolderA");
        assert_eq!(panels.right.tabs[1].snapshot.path, r"D:\Videos\FolderB");
        assert_eq!(
            panels.right.tabs[0].snapshot.view_mode,
            crate::domain::file_entry::ViewMode::ColumnList
        );
        assert_eq!(
            panels.right.tabs[1].snapshot.view_mode,
            crate::domain::file_entry::ViewMode::Miller
        );
    }
}
