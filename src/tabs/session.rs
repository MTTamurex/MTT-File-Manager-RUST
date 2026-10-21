use super::{TabManager, TabState, MAX_PANEL_TABS, MAX_TABS};
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use serde::{Deserialize, Serialize};

const SESSION_VERSION: u32 = 1;
const MAX_SESSION_BYTES: usize = 1024 * 1024;

/// Persistable subset of the live tab manager. Folder listings and rendering
/// resources are intentionally rebuilt after startup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabManagerSession {
    version: u32,
    pub(crate) active_tab: usize,
    pub(crate) tabs: Vec<SessionTab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionTab {
    #[serde(default)]
    pub(crate) pinned: bool,
    pub(crate) path: String,
    pub(crate) view_mode: String,
    pub(crate) sort_mode: String,
    pub(crate) sort_descending: bool,
    pub(crate) folders_position: String,
    pub(crate) group_mode: String,
    pub(crate) group_descending: bool,
    pub(crate) show_left_sidebar: bool,
    pub(crate) show_preview_panel: bool,
    pub(crate) collapse_tags: bool,
    pub(crate) active_tag_filter: Option<i64>,
    pub(crate) dual_panel_enabled: bool,
    pub(crate) dual_panel_active: String,
    pub(crate) dual_panel_split_ratio: f32,
    pub(crate) dual_panel_active_column_widths: Option<SessionColumnWidths>,
    pub(crate) dual_panel_tabs: Option<SessionDualPanelTabs>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionColumnWidths {
    pub(crate) values: [f32; 12],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionDualPanelTabs {
    pub(crate) left: SessionPanelTabSet,
    pub(crate) right: SessionPanelTabSet,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionPanelTabSet {
    pub(crate) active_tab: usize,
    pub(crate) tabs: Vec<SessionPanelTab>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionPanelTab {
    #[serde(default)]
    pub(crate) pinned: bool,
    pub(crate) id: usize,
    pub(crate) path: String,
    pub(crate) view_mode: String,
    pub(crate) sort_mode: String,
    pub(crate) sort_descending: bool,
    pub(crate) folders_position: String,
    pub(crate) group_mode: String,
    pub(crate) group_descending: bool,
    pub(crate) active_tag_filter: Option<i64>,
    pub(crate) column_widths: SessionColumnWidths,
}

impl TabManagerSession {
    pub(crate) fn from_tab_manager(manager: &TabManager) -> Self {
        Self {
            version: SESSION_VERSION,
            active_tab: manager.active_tab,
            tabs: manager.tabs.iter().map(SessionTab::from_tab).collect(),
        }
    }

    pub(crate) fn from_json(value: &str) -> Option<Self> {
        if value.len() > MAX_SESSION_BYTES {
            return None;
        }
        let mut session: Self = serde_json::from_str(value).ok()?;
        if session.version != SESSION_VERSION
            || session.tabs.is_empty()
            || session.tabs.len() > MAX_TABS
            || session.active_tab >= session.tabs.len()
        {
            return None;
        }

        for tab in &mut session.tabs {
            if !tab.dual_panel_split_ratio.is_finite() {
                tab.dual_panel_split_ratio = 0.5;
            }
            tab.dual_panel_split_ratio = tab.dual_panel_split_ratio.clamp(0.2, 0.8);
            if let Some(widths) = tab.dual_panel_active_column_widths.as_mut() {
                widths.normalize();
            }
            if let Some(panel_tabs) = &mut tab.dual_panel_tabs {
                if !valid_panel_tab_set(&panel_tabs.left)
                    || !valid_panel_tab_set(&panel_tabs.right)
                    || panel_tabs.left.tabs.len() + panel_tabs.right.tabs.len() > MAX_PANEL_TABS
                {
                    tab.dual_panel_tabs = None;
                    tab.dual_panel_enabled = false;
                }
                if let Some(panel_tabs) = &mut tab.dual_panel_tabs {
                    for panel_tab in panel_tabs
                        .left
                        .tabs
                        .iter_mut()
                        .chain(panel_tabs.right.tabs.iter_mut())
                    {
                        panel_tab.column_widths.normalize();
                    }
                }
            }
        }
        Some(session)
    }

    pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub(crate) fn active_path(&self) -> &str {
        &self.tabs[self.active_tab].path
    }

    pub(crate) fn active_tab_record(&self) -> &SessionTab {
        &self.tabs[self.active_tab]
    }

    pub(crate) fn replace_invalid_paths(&mut self, is_valid: impl Fn(&str) -> bool) {
        for tab in &mut self.tabs {
            if !is_valid(&tab.path) {
                tab.path = COMPUTER_VIEW_ID.to_string();
                tab.active_tag_filter = None;
            }
            if let Some(panel_tabs) = &mut tab.dual_panel_tabs {
                for panel_tab in panel_tabs
                    .left
                    .tabs
                    .iter_mut()
                    .chain(panel_tabs.right.tabs.iter_mut())
                {
                    if !is_valid(&panel_tab.path) {
                        panel_tab.path = COMPUTER_VIEW_ID.to_string();
                        panel_tab.active_tag_filter = None;
                    }
                }
            }
        }
    }

    pub(crate) fn replace_invalid_tag_filters(&mut self, is_valid: impl Fn(i64) -> bool) {
        for tab in &mut self.tabs {
            tab.active_tag_filter = tab.active_tag_filter.filter(|id| is_valid(*id));
            if let Some(panel_tabs) = &mut tab.dual_panel_tabs {
                for panel_tab in panel_tabs
                    .left
                    .tabs
                    .iter_mut()
                    .chain(panel_tabs.right.tabs.iter_mut())
                {
                    panel_tab.active_tag_filter =
                        panel_tab.active_tag_filter.filter(|id| is_valid(*id));
                }
            }
        }
    }

    pub(crate) fn into_tab_manager(self) -> TabManager {
        let tabs: Vec<TabState> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(id, saved)| saved.to_tab(id))
            .collect();
        let next_id = tabs
            .iter()
            .map(|tab| tab.id)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        TabManager {
            tabs,
            active_tab: self.active_tab,
            next_id,
            closed_tabs: Vec::new(),
        }
    }
}

fn valid_panel_tab_set(set: &SessionPanelTabSet) -> bool {
    !set.tabs.is_empty() && set.tabs.len() <= MAX_PANEL_TABS && set.active_tab < set.tabs.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::ViewMode;

    #[test]
    fn session_round_trips_outer_tab_order_and_active_index() {
        let mut manager = TabManager::new_at_path(r"C:\FolderOne");
        manager.new_tab_at(r"D:\FolderTwo");
        manager.switch_to(0);
        manager.active_mut().view_mode = ViewMode::List;
        manager.tabs[1].sort_descending = true;

        let serialized = TabManagerSession::from_tab_manager(&manager)
            .to_json()
            .unwrap();
        let restored = TabManagerSession::from_json(&serialized).unwrap();
        let restored_manager = restored.into_tab_manager();

        assert_eq!(restored_manager.active_tab, 0);
        assert_eq!(restored_manager.tabs.len(), 2);
        assert_eq!(restored_manager.tabs[0].path, r"C:\FolderOne");
        assert_eq!(restored_manager.tabs[0].view_mode, ViewMode::List);
        assert_eq!(restored_manager.tabs[1].path, r"D:\FolderTwo");
        assert!(restored_manager.tabs[1].sort_descending);
    }

    #[test]
    fn dual_panel_session_keeps_focused_side_and_panel_tab_selections() {
        let manager = TabManager::new();
        let mut session = TabManagerSession::from_tab_manager(&manager);
        let panel_tab = |id, path: &str| SessionPanelTab {
            id,
            pinned: false,
            path: path.to_owned(),
            view_mode: "grid".to_owned(),
            sort_mode: "name".to_owned(),
            sort_descending: false,
            folders_position: "first".to_owned(),
            group_mode: "none".to_owned(),
            group_descending: false,
            active_tag_filter: None,
            column_widths: SessionColumnWidths {
                values: [120.0; 12],
            },
        };
        let workspace = &mut session.tabs[0];
        workspace.dual_panel_enabled = true;
        workspace.dual_panel_active = "right".to_owned();
        workspace.dual_panel_split_ratio = 0.65;
        workspace.dual_panel_active_column_widths = Some(SessionColumnWidths {
            values: [135.0; 12],
        });
        workspace.dual_panel_tabs = Some(SessionDualPanelTabs {
            left: SessionPanelTabSet {
                active_tab: 1,
                tabs: vec![
                    panel_tab(0, r"C:\Users\user\Documents\Project"),
                    panel_tab(2, r"D:\Work\Archive"),
                ],
            },
            right: SessionPanelTabSet {
                active_tab: 0,
                tabs: vec![panel_tab(1, r"E:\Media\Library")],
            },
        });
        workspace.dual_panel_tabs.as_mut().unwrap().left.tabs[0].pinned = true;

        let restored = TabManagerSession::from_json(&session.to_json().unwrap()).unwrap();
        let workspace = &restored.tabs[0];
        let tabs = workspace.dual_panel_tabs.as_ref().unwrap();

        assert_eq!(workspace.dual_panel_active, "right");
        assert!((workspace.dual_panel_split_ratio - 0.65).abs() < f32::EPSILON);
        assert_eq!(
            workspace.dual_panel_active_column_widths.unwrap().values[0],
            135.0
        );
        assert_eq!(tabs.left.active_tab, 1);
        assert!(tabs.left.tabs[0].pinned);
        assert!(!tabs.left.tabs[1].pinned);
        assert_eq!(tabs.left.tabs[1].path, r"D:\Work\Archive");
        assert_eq!(tabs.left.tabs[1].column_widths.values[0], 120.0);
        assert_eq!(tabs.right.active_tab, 0);
    }

    #[test]
    fn invalid_or_oversized_sessions_are_rejected() {
        assert!(TabManagerSession::from_json("not-json").is_none());
        assert!(TabManagerSession::from_json(&"x".repeat(MAX_SESSION_BYTES + 1)).is_none());

        let wrong_version = r#"{"version":99,"active_tab":0,"tabs":[{}]}"#;
        assert!(TabManagerSession::from_json(wrong_version).is_none());
    }

    #[test]
    fn invalid_persisted_paths_are_replaced_without_discarding_other_tabs() {
        let mut manager = TabManager::new_at_path(r"C:\FolderOne");
        manager.new_tab_at(r"D:\FolderTwo");
        let mut session = TabManagerSession::from_tab_manager(&manager);

        session.replace_invalid_paths(|path| path == r"C:\FolderOne");
        let restored = session.into_tab_manager();

        assert_eq!(restored.tabs.len(), 2);
        assert_eq!(restored.tabs[0].path, r"C:\FolderOne");
        assert_eq!(restored.tabs[1].path, COMPUTER_VIEW_ID);
    }
}
