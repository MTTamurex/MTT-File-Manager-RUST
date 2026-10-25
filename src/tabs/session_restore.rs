use super::session::{
    SessionColumnWidths, SessionDualPanelTabs, SessionPanelTab, SessionPanelTabSet, SessionTab,
    TabManagerSession,
};
use super::{DualPanelTabs, PanelTab, PanelTabSet, TabManager, TabState};
use crate::app::dual_panel::{ActivePanel, PanelListColumnWidths, PanelSnapshot};
use crate::app::state::ImageViewerApp;
use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{FoldersPosition, GroupMode, SortMode, ViewMode};
use crate::domain::file_tag::FileTag;
use crate::domain::special_paths::{tag_id_from_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use rustc_hash::FxHashMap;

impl TabManager {
    /// Resolve tag labels before the first frame, including tabs not yet activated.
    pub(crate) fn restore_tag_titles(&mut self, definitions: &FxHashMap<i64, FileTag>) {
        for tab in &mut self.tabs {
            if let Some(tag_id) = tag_id_from_view_path(&tab.path) {
                let name = definitions
                    .get(&tag_id)
                    .map(|tag| tag.name.clone())
                    .unwrap_or_else(|| tag_id.to_string());
                tab.title = rust_i18n::t!("tags.filter_active", name = name).to_string();
                tab.path_input = tab.title.clone();
            }
        }
    }
}

impl SessionTab {
    pub(crate) fn from_tab(tab: &TabState) -> Self {
        Self {
            pinned: tab.pinned,
            path: tab.path.clone(),
            view_mode: tab.view_mode.preference_value().to_string(),
            sort_mode: sort_mode_preference(tab.sort_mode).to_string(),
            sort_descending: tab.sort_descending,
            folders_position: folders_position_preference(tab.folders_position).to_string(),
            group_mode: tab.group_mode.preference_value().to_string(),
            group_descending: tab.group_descending,
            show_left_sidebar: tab.show_left_sidebar,
            show_preview_panel: tab.show_preview_panel,
            collapse_tags: tab.collapse_tags,
            active_tag_filter: tab.active_tag_filter,
            dual_panel_enabled: tab.dual_panel_enabled,
            dual_panel_active: active_panel_preference(tab.dual_panel_active).to_string(),
            dual_panel_split_ratio: if tab.dual_panel_split_ratio.is_finite() {
                tab.dual_panel_split_ratio.clamp(0.2, 0.8)
            } else {
                0.5
            },
            dual_panel_active_column_widths: tab
                .dual_panel_active_list_column_widths
                .map(SessionColumnWidths::from_widths),
            dual_panel_tabs: tab
                .dual_panel_tabs
                .as_ref()
                .map(SessionDualPanelTabs::from_tabs),
        }
    }

    pub(crate) fn to_tab(&self, id: usize) -> TabState {
        let mut tab = if self.path == COMPUTER_VIEW_ID {
            TabState::new_at_computer(id)
        } else {
            TabState::new_at_path(id, &self.path)
        };
        tab.path = self.path.clone();
        tab.pinned = self.pinned;
        tab.path_input = self.path.clone();
        tab.is_computer_view = self.path == COMPUTER_VIEW_ID;
        tab.is_recycle_bin_view = self.path == RECYCLE_BIN_VIEW_ID;
        tab.navigation = NavigationHistory::new(self.path.clone());
        tab.view_mode = ViewMode::from_preference(&self.view_mode);
        tab.sort_mode = parse_sort_mode(&self.sort_mode);
        tab.sort_descending = self.sort_descending;
        tab.folders_position = parse_folders_position(&self.folders_position);
        tab.group_mode = GroupMode::from_preference(&self.group_mode);
        tab.group_descending = self.group_descending;
        tab.show_left_sidebar = self.show_left_sidebar;
        tab.show_preview_panel = self.show_preview_panel;
        tab.collapse_tags = self.collapse_tags;
        tab.active_tag_filter = self.active_tag_filter;
        tab.dual_panel_enabled = self.dual_panel_enabled;
        tab.dual_panel_active = parse_active_panel(&self.dual_panel_active);
        tab.dual_panel_split_ratio = self.dual_panel_split_ratio;
        tab.dual_panel_active_list_column_widths = self
            .dual_panel_active_column_widths
            .map(SessionColumnWidths::to_widths);
        tab
    }
}

impl SessionDualPanelTabs {
    fn from_tabs(tabs: &DualPanelTabs) -> Self {
        Self {
            left: SessionPanelTabSet::from_tab_set(&tabs.left),
            right: SessionPanelTabSet::from_tab_set(&tabs.right),
        }
    }
}

impl SessionPanelTabSet {
    fn from_tab_set(set: &PanelTabSet) -> Self {
        Self {
            active_tab: set.active_tab,
            tabs: set
                .tabs
                .iter()
                .map(|tab| {
                    let mut saved = SessionPanelTab::from_snapshot(tab.id, &tab.snapshot);
                    saved.pinned = tab.pinned;
                    saved
                })
                .collect(),
        }
    }
}

impl SessionPanelTab {
    fn from_snapshot(id: usize, snapshot: &PanelSnapshot) -> Self {
        Self {
            id,
            pinned: false,
            path: snapshot.path.clone(),
            view_mode: snapshot.view_mode.preference_value().to_string(),
            sort_mode: sort_mode_preference(snapshot.sort_mode).to_string(),
            sort_descending: snapshot.sort_descending,
            folders_position: folders_position_preference(snapshot.folders_position).to_string(),
            group_mode: snapshot.group_mode.preference_value().to_string(),
            group_descending: snapshot.group_descending,
            active_tag_filter: snapshot.active_tag_filter,
            column_widths: SessionColumnWidths::from_widths(snapshot.list_column_widths),
        }
    }

    fn apply_to_snapshot(&self, snapshot: &mut PanelSnapshot) {
        snapshot.path = self.path.clone();
        snapshot.path_input = self.path.clone();
        snapshot.is_computer_view = self.path == COMPUTER_VIEW_ID;
        snapshot.is_recycle_bin_view = self.path == RECYCLE_BIN_VIEW_ID;
        snapshot.navigation = NavigationHistory::new(self.path.clone());
        snapshot.view_mode = ViewMode::from_preference(&self.view_mode);
        snapshot.sort_mode = parse_sort_mode(&self.sort_mode);
        snapshot.sort_descending = self.sort_descending;
        snapshot.folders_position = parse_folders_position(&self.folders_position);
        snapshot.group_mode = GroupMode::from_preference(&self.group_mode);
        snapshot.group_descending = self.group_descending;
        snapshot.active_tag_filter = self.active_tag_filter;
        snapshot.list_column_widths = self.column_widths.to_widths();
        // Refit using this pane's actual width once its restored listing is ready.
        snapshot.pending_list_column_autofit = true;
        snapshot.items = std::sync::Arc::new(Vec::new());
        snapshot.all_items = std::sync::Arc::new(Vec::new());
        snapshot.items_snapshot_compact = false;
        snapshot.total_items = 0;
        snapshot.is_loading_folder = false;
        snapshot.folder_load_error = None;
        snapshot.selected_item = None;
        snapshot.selected_file = None;
        snapshot.multi_selection.clear();
        snapshot.selection_anchor = None;
        snapshot.rectangle_selection_state = None;
        snapshot.search_query.clear();
        snapshot.group_projection = std::sync::Arc::new(Default::default());
        snapshot.collapsed_groups_by_context.clear();
        snapshot.selected_thumbnail = None;
        snapshot.selected_metadata = None;
        snapshot.selected_gif = None;
        snapshot.loaded_path.clear();
        snapshot.renaming_state = None;
        snapshot.focus_rename = false;
        snapshot.pending_all_items_clear = false;
        snapshot.hold_visible_items_until_load_complete = false;
        snapshot.pending_items_rebuild = false;
        snapshot.pending_items_count = 0;
        snapshot.inactive_final_items_rebuild_pending = false;
        snapshot.stale_items_snapshot = None;
        snapshot.scroll_offset_x = 0.0;
        snapshot.scroll_offset_y = 0.0;
        snapshot.scroll_to_selected = false;
        snapshot.visible_index_range = None;
        snapshot.visible_paths_cache.clear();
        snapshot.visible_group_paths.clear();
        snapshot.visible_range_cached = None;
        snapshot.current_folder_locked = false;
        snapshot.miller_columns = Default::default();
        snapshot.loading_started_at = std::time::Instant::now();
        snapshot.last_items_rebuild = std::time::Instant::now();
    }
}

impl SessionColumnWidths {
    fn from_widths(widths: PanelListColumnWidths) -> Self {
        let mut session = Self {
            values: [
                widths.name,
                widths.date,
                widths.type_col,
                widths.size,
                widths.onedrive_name,
                widths.onedrive_date,
                widths.onedrive_type,
                widths.onedrive_size,
                widths.onedrive_status,
                widths.computer_name,
                widths.computer_total,
                widths.computer_free,
            ],
        };
        session.normalize();
        session
    }

    fn to_widths(self) -> PanelListColumnWidths {
        let [name, date, type_col, size, onedrive_name, onedrive_date, onedrive_type, onedrive_size, onedrive_status, computer_name, computer_total, computer_free] =
            self.values;
        PanelListColumnWidths {
            name,
            date,
            type_col,
            size,
            onedrive_name,
            onedrive_date,
            onedrive_type,
            onedrive_size,
            onedrive_status,
            computer_name,
            computer_total,
            computer_free,
        }
    }

    pub(crate) fn normalize(&mut self) {
        for width in &mut self.values {
            *width = if width.is_finite() {
                (*width).clamp(24.0, 4096.0)
            } else {
                180.0
            };
        }
    }
}

impl TabManagerSession {
    /// Materialize runtime-only panel snapshots after the app and workers have
    /// been constructed. Restored listings are intentionally reloaded later.
    pub(crate) fn restore_panel_tabs(&self, app: &mut ImageViewerApp) {
        let restored: Vec<Option<DualPanelTabs>> = self
            .tabs
            .iter()
            .map(|tab| {
                tab.dual_panel_tabs.as_ref().map(|saved| {
                    let left = restore_panel_tab_set(app, &saved.left);
                    let right = restore_panel_tab_set(app, &saved.right);
                    DualPanelTabs::from_panel_tab_sets(left, right)
                })
            })
            .collect();

        for (tab, panel_tabs) in app.tab_manager.tabs.iter_mut().zip(restored) {
            tab.dual_panel_tabs = panel_tabs;
        }
    }
}

fn restore_panel_tab_set(app: &mut ImageViewerApp, saved: &SessionPanelTabSet) -> PanelTabSet {
    let tabs = saved
        .tabs
        .iter()
        .map(|saved_tab| {
            let mut snapshot = app.new_panel_tab_snapshot(&saved_tab.path);
            saved_tab.apply_to_snapshot(&mut snapshot);
            PanelTab {
                id: saved_tab.id,
                pinned: saved_tab.pinned,
                snapshot,
            }
        })
        .collect::<Vec<_>>();
    PanelTabSet {
        active_tab: saved.active_tab.min(tabs.len().saturating_sub(1)),
        tabs,
    }
}

fn sort_mode_preference(mode: SortMode) -> String {
    mode.preference_value()
}

fn parse_sort_mode(value: &str) -> SortMode {
    SortMode::from_preference(value)
}

fn folders_position_preference(position: FoldersPosition) -> &'static str {
    match position {
        FoldersPosition::First => "first",
        FoldersPosition::Last => "last",
        FoldersPosition::Mixed => "mixed",
    }
}

fn parse_folders_position(value: &str) -> FoldersPosition {
    match value {
        "last" => FoldersPosition::Last,
        "mixed" => FoldersPosition::Mixed,
        _ => FoldersPosition::First,
    }
}

fn active_panel_preference(panel: ActivePanel) -> &'static str {
    match panel {
        ActivePanel::Left => "left",
        ActivePanel::Right => "right",
    }
}

fn parse_active_panel(value: &str) -> ActivePanel {
    match value {
        "right" => ActivePanel::Right,
        _ => ActivePanel::Left,
    }
}

#[cfg(test)]
mod tag_title_tests {
    use super::*;
    use crate::domain::file_tag::TagColor;
    use crate::domain::special_paths::tag_view_path;

    #[test]
    fn restored_inactive_tag_tabs_have_labels_before_activation() {
        let mut manager = TabManager::new();
        manager
            .tabs
            .push(TabState::new_at_path(1, &tag_view_path(2)));
        manager
            .tabs
            .push(TabState::new_at_path(2, &tag_view_path(9)));
        let session = TabManagerSession::from_tab_manager(&manager);
        let mut restored = session.into_tab_manager();
        let definitions = [(2, "Work"), (9, "Archive")]
            .into_iter()
            .map(|(id, name)| {
                (
                    id,
                    FileTag {
                        id,
                        name: name.to_string(),
                        color: TagColor::Red,
                        position: id,
                    },
                )
            })
            .collect();
        let computer_title = restored.tabs[0].title.clone();

        restored.restore_tag_titles(&definitions);

        assert_eq!(restored.active_tab, 0);
        assert_eq!(restored.tabs[0].title, computer_title);
        for (index, id, name) in [(1, 2, "Work"), (2, 9, "Archive")] {
            let tab = &restored.tabs[index];
            let expected = rust_i18n::t!("tags.filter_active", name = name).to_string();
            assert_eq!(tab.title, expected);
            assert_eq!(tab.path_input, expected);
            assert_eq!(tab.path, tag_view_path(id));
            assert!(tab.items.is_empty());
        }
    }
}
