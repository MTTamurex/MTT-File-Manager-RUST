use crate::app::dual_panel::{ActivePanel, PanelListColumnWidths, PanelSnapshot};
use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::COMPUTER_VIEW_ID;
use crate::tabs::DualPanelTabs;
use std::path::Path;
use std::sync::Arc;

impl ImageViewerApp {
    pub fn sync_to_tab(&mut self) {
        let current_tag_title =
            self.tag_view_display_name_for_path(&self.navigation_state.current_path);
        let panel_tab_snapshots = if self.dual_panel_enabled {
            self.dual_panel_inactive_state.as_ref().map(|inactive| {
                let mut active = PanelSnapshot::from_app(self);
                active.compact_for_storage();
                let mut inactive = inactive.clone();
                inactive.compact_for_storage();
                (self.dual_panel_active, active, Some(inactive))
            })
        } else if self.tab_manager.active().dual_panel_tabs.is_some() {
            let mut active = PanelSnapshot::from_app(self);
            active.compact_for_storage();
            Some((self.dual_panel_active, active, None))
        } else {
            None
        };
        let active = self.tab_manager.active_mut();
        active.path = self.navigation_state.current_path.clone();
        active.path_input = self.navigation_state.path_input.clone();
        active.is_computer_view = self.navigation_state.is_computer_view;
        active.is_recycle_bin_view = self.navigation_state.is_recycle_bin_view;
        active.navigation = self.navigation_state.navigation.clone();
        let compact_items_snapshot = self.search_query.is_empty()
            && !self.is_loading_folder
            && !self.pending_items_rebuild
            && self.items.len() == self.all_items.len();
        if compact_items_snapshot {
            active.all_items = self.items.clone();
            active.items = Arc::new(Vec::new());
            active.items_snapshot_compact = true;
        } else {
            active.all_items = self.all_items.clone();
            active.items = self.items.clone();
            active.items_snapshot_compact = false;
        }
        active.selected_item = self.selected_item;
        active.generation = self.generation;
        active.is_loading_folder = self.is_loading_folder;
        active.items_revision = self.items_revision;
        active.pending_items_rebuild = self.pending_items_rebuild;
        active.pending_items_count = self.pending_items_count;
        active.inactive_final_items_rebuild_pending = self.inactive_final_items_rebuild_pending;
        active.selected_file = self.selected_file.clone();
        active.selected_thumbnail = None;
        active.selected_gif = None;
        active.selected_metadata = self.selected_metadata.clone();
        active.search_query = self.search_query.clone();
        active.scroll_to_selected = self.scroll_to_selected;
        active.scroll_offset_y = self.scroll_offset_y;
        active.scroll_offset_x = self.scroll_offset_x;
        active.total_items = self.total_items;
        active.view_mode = self.view_mode;
        // Sync also runs without switching tabs, so keep the live selection.
        active.multi_selection = self.multi_selection.clone();
        active.sort_mode = self.sort_mode;
        active.sort_descending = self.sort_descending;
        active.folders_position = self.folders_position;
        active.group_mode = self.group_mode;
        active.group_descending = self.group_descending;
        active.group_projection = self.group_projection.clone();
        active.collapsed_groups_by_context = self.collapsed_groups_by_context.clone();
        active.show_left_sidebar = self.show_left_sidebar;
        active.show_preview_panel = self.show_preview_panel;
        active.collapse_quick_access = self.collapse_quick_access;
        active.collapse_cloud_drives = self.collapse_cloud_drives;
        active.collapse_local_disks = self.collapse_local_disks;
        active.collapse_network_drives = self.collapse_network_drives;
        active.collapse_network_locations = self.collapse_network_locations;
        active.active_tag_filter = self.active_tag_filter;
        active.collapse_tags = self.collapse_tags;
        active.dual_panel_enabled = self.dual_panel_enabled;
        active.dual_panel_active = self.dual_panel_active;
        active.dual_panel_split_ratio = self.layout.dual_panel_split_ratio;
        active.dual_panel_active_list_column_widths = self
            .dual_panel_enabled
            .then(|| PanelListColumnWidths::from_layout(&self.layout));
        active.dual_panel_inactive_state =
            self.dual_panel_inactive_state.clone().map(|mut snapshot| {
                snapshot.compact_for_storage();
                snapshot
            });
        if active.dual_panel_tabs.is_none() {
            if let Some((current_panel, current, Some(inactive))) = &panel_tab_snapshots {
                let (left, right) = match current_panel {
                    ActivePanel::Left => (current.clone(), inactive.clone()),
                    ActivePanel::Right => (inactive.clone(), current.clone()),
                };
                active.dual_panel_tabs = Some(DualPanelTabs::new(left, right));
            }
        }
        if let (Some(tabs), Some((current_panel, current, inactive))) =
            (&mut active.dual_panel_tabs, panel_tab_snapshots)
        {
            let (current_tabs, other_tabs) = match current_panel {
                ActivePanel::Left => (&mut tabs.left, &mut tabs.right),
                ActivePanel::Right => (&mut tabs.right, &mut tabs.left),
            };
            current_tabs.sync_active_snapshot(current, self.dual_panel_enabled);
            if let Some(inactive) = inactive {
                other_tabs.sync_active_snapshot(inactive, self.dual_panel_enabled);
            }
        }
        active.sidebar_expanded = self.sidebar_tree.snapshot_expanded();
        active.sidebar_scroll_y = self.sidebar_tree.snapshot_scroll_y();
        if active.is_computer_view {
            active.title = COMPUTER_VIEW_ID.to_string();
        } else if let Some(title) = current_tag_title {
            active.title = title;
        } else {
            active.title = Path::new(&active.path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| active.path.clone());
        }
    }
}
