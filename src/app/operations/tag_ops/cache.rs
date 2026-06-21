use super::*;
use crate::app::state::ImageViewerApp;
use crate::application::sorting;
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
        // Check if the active panel tag view needs a full reload because a
        // newly tagged file is not in all_items yet.
        if let Some(tag_id) = tag_id_from_view_path(&self.navigation_state.current_path) {
            if self.tag_view_needs_reload(tag_id) {
                self.setup_tag_view(tag_id);
                self.ui_ctx.request_repaint();
                return;
            }
        }

        self.filter_items();

        // Re-filter the inactive panel if it's showing a tag view.
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            if let Some(tag_id) = tag_id_from_view_path(&snapshot.path) {
                let needs_reload = {
                    let paths_with_tag: Vec<&PathBuf> = self
                        .tag_assignments
                        .iter()
                        .filter(|(_, ids)| ids.contains(&tag_id))
                        .map(|(path, _)| path)
                        .collect();
                    let view_paths: FxHashSet<&PathBuf> =
                        snapshot.all_items.iter().map(|e| &e.path).collect();
                    paths_with_tag.iter().any(|p| !view_paths.contains(p))
                };
                if needs_reload {
                    self.with_inactive_panel(|app| {
                        app.setup_tag_view(tag_id);
                    });
                } else {
                    let snapshot = self.dual_panel_inactive_state.as_mut().unwrap();
                    Self::refilter_snapshot_tag_view(snapshot, &self.tag_assignments);
                }
            }
        }

        self.ui_ctx.request_repaint();
    }

    /// Returns true if a tagged file is missing from the active panel's all_items,
    /// meaning the tag view must be fully reloaded to include it.
    fn tag_view_needs_reload(&self, tag_id: i64) -> bool {
        let view_paths: FxHashSet<&PathBuf> = self.all_items.iter().map(|e| &e.path).collect();
        self.tag_assignments
            .iter()
            .filter(|(_, ids)| ids.contains(&tag_id))
            .any(|(path, _)| !view_paths.contains(path))
    }

    fn refilter_snapshot_tag_view(
        snapshot: &mut crate::app::dual_panel::PanelSnapshot,
        tag_assignments: &FxHashMap<PathBuf, Vec<i64>>,
    ) {
        // When compacted, all_items holds the real data and items is empty.
        // When not compacted, all_items is the authoritative source too.
        let source = &snapshot.all_items;

        let has_query = !snapshot.search_query.is_empty();
        let has_tag = snapshot.active_tag_filter.is_some();

        if !has_query && !has_tag {
            snapshot.items = source.clone();
            snapshot.items_snapshot_compact = false;
        } else if let Some(mut filtered) = sorting::filter_items_opt_with_tags(
            source,
            &snapshot.search_query,
            snapshot.active_tag_filter,
            tag_assignments,
        ) {
            sorting::sort_items(
                &mut filtered,
                snapshot.sort_mode,
                snapshot.sort_descending,
                snapshot.folders_position,
            );
            snapshot.items = Arc::new(filtered);
            snapshot.items_snapshot_compact = false;
        } else {
            let mut all = source.as_ref().clone();
            sorting::sort_items(
                &mut all,
                snapshot.sort_mode,
                snapshot.sort_descending,
                snapshot.folders_position,
            );
            snapshot.items = Arc::new(all);
            snapshot.items_snapshot_compact = false;
        }

        snapshot.total_items = snapshot.items.len();
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

        // NOTE: The active tab's inactive panel snapshot is NOT cleared here.
        // It is re-filtered in-place by refresh_visible_items_after_tag_change()
        // to avoid a visual flash from clearing and reloading.
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

    /// Removes items from tag views whose files no longer exist on disk.
    ///
    /// Called when the app regains focus to clean up entries for files that were
    /// deleted externally (e.g. via Windows Explorer) while a tag view was open.
    pub fn purge_missing_files_from_tag_views(&mut self) {
        let mut needs_refresh = false;

        // Validate active panel tag view.
        if tag_id_from_view_path(&self.navigation_state.current_path).is_some() {
            let missing: Vec<PathBuf> = self
                .all_items
                .iter()
                .filter(|item| !item.path.exists())
                .map(|item| item.path.clone())
                .collect();
            if !missing.is_empty() {
                self.reconcile_garbage_collected_tag_assignments(&missing);
                needs_refresh = true;
            }
        }

        // Validate inactive panel tag view.
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            if tag_id_from_view_path(&snapshot.path).is_some() {
                let missing: Vec<PathBuf> = snapshot
                    .all_items
                    .iter()
                    .filter(|item| !item.path.exists())
                    .map(|item| item.path.clone())
                    .collect();
                if !missing.is_empty() {
                    self.reconcile_garbage_collected_tag_assignments(&missing);
                    needs_refresh = true;
                }
            }
        }

        if needs_refresh {
            self.ui_ctx.request_repaint();
        }
    }
}
