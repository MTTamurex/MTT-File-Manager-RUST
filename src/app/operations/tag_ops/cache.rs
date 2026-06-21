use super::*;
use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::tag_id_from_view_path;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;
use std::sync::Arc;

impl ImageViewerApp {
    pub(super) fn recompute_tag_counts_from_assignments(&mut self) {
        let mut counts = FxHashMap::default();
        for tag_ids in self.tag_assignments.values() {
            for tag_id in tag_ids {
                *counts.entry(*tag_id).or_insert(0) += 1;
            }
        }
        self.tag_counts = counts;
    }

    pub(super) fn refresh_visible_items_after_tag_change(&mut self) {
        self.filter_items();

        // Reload inactive panel if it's showing a tag view whose assignments changed.
        // Without this, the inactive panel's items remain empty after
        // invalidate_cached_tag_views_for_tags() clears them.
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            if let Some(tag_id) = tag_id_from_view_path(&snapshot.path) {
                self.with_inactive_panel(|app| {
                    app.setup_tag_view(tag_id);
                });
            }
        }

        self.ui_ctx.request_repaint();
    }

    fn clear_cached_tag_tab_items(tab: &mut crate::tabs::TabState) {
        tab.items = Arc::new(Vec::new());
        tab.all_items = Arc::new(Vec::new());
        tab.items_snapshot_compact = false;
        tab.total_items = 0;
        tab.selected_item = None;
        tab.selected_file = None;
        tab.selected_thumbnail = None;
        tab.selected_metadata = None;
        tab.selected_gif = None;
        tab.multi_selection.clear();
    }

    fn clear_cached_tag_snapshot_items(snapshot: &mut crate::app::dual_panel::PanelSnapshot) {
        snapshot.items = Arc::new(Vec::new());
        snapshot.all_items = Arc::new(Vec::new());
        snapshot.items_snapshot_compact = false;
        snapshot.total_items = 0;
        snapshot.selected_item = None;
        snapshot.selected_file = None;
        snapshot.selected_thumbnail = None;
        snapshot.selected_metadata = None;
        snapshot.selected_gif = None;
        snapshot.multi_selection.clear();
    }

    pub(super) fn invalidate_cached_tag_views_for_tags(&mut self, tag_ids: &FxHashSet<i64>) {
        if tag_ids.is_empty() {
            return;
        }

        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter_mut().enumerate() {
            if index != active_tab
                && tag_id_from_view_path(&tab.path).is_some_and(|id| tag_ids.contains(&id))
            {
                Self::clear_cached_tag_tab_items(tab);
            }

            if let Some(snapshot) = tab.dual_panel_inactive_state.as_mut() {
                if tag_id_from_view_path(&snapshot.path).is_some_and(|id| tag_ids.contains(&id)) {
                    Self::clear_cached_tag_snapshot_items(snapshot);
                }
            }
        }

        if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
            if tag_id_from_view_path(&snapshot.path).is_some_and(|id| tag_ids.contains(&id)) {
                Self::clear_cached_tag_snapshot_items(snapshot);
            }
        }
    }

    pub fn reconcile_garbage_collected_tag_assignments(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let changed_tags: FxHashSet<i64> = self
            .tag_assignments
            .iter()
            .filter(|(assigned_path, _)| {
                paths
                    .iter()
                    .any(|path| tag_assignment_path_matches(assigned_path, path))
            })
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        if changed_tags.is_empty() {
            return;
        }

        let assignments = Arc::make_mut(&mut self.tag_assignments);
        assignments.retain(|assigned_path, _| {
            !paths
                .iter()
                .any(|path| tag_assignment_path_matches(assigned_path, path))
        });
        self.recompute_tag_counts_from_assignments();
        self.invalidate_cached_tag_views_for_tags(&changed_tags);
        self.refresh_visible_items_after_tag_change();
    }
}
