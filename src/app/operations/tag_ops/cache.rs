use super::*;
use crate::app::state::ImageViewerApp;
use crate::application::sorting;
use crate::domain::file_tag;
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
            self.prune_active_tag_view_items_without_tag(tag_id);
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
                    Self::refilter_snapshot_tag_view(
                        snapshot,
                        self.tag_assignments_normalized.as_ref(),
                    );
                }
            }
        }

        self.ui_ctx.request_repaint();
    }

    fn prune_active_tag_view_items_without_tag(&mut self, tag_id: i64) {
        let tag_assignments = self.tag_assignments_normalized.clone();
        self.all_items_mut()
            .retain(|item| file_tag::path_has_tag(tag_assignments.as_ref(), &item.path, tag_id));
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
        tag_assignments: &FxHashMap<String, Vec<i64>>,
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

        // Active views are reloaded by the caller when required.
    }

    pub(crate) fn refresh_tag_views_after_drive_change(&mut self) {
        if self.tag_definitions.is_empty() {
            return;
        }

        let tag_ids: FxHashSet<i64> = self.tag_definitions.keys().copied().collect();
        self.invalidate_cached_tag_views_for_tags(&tag_ids);
        self.reload_visible_tag_views();
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
        self.sync_tag_assignments_normalized();
        self.prune_paths_from_loaded_tag_views(paths);
        self.refresh_visible_items_after_tag_change();
    }

    fn retain_tag_view_items(
        items: &mut Arc<Vec<crate::domain::file_entry::FileEntry>>,
        removed_path_keys: &FxHashSet<String>,
    ) {
        Arc::make_mut(items)
            .retain(|item| !removed_path_keys.contains(&normalize_path_text(&item.path)));
    }

    fn prune_paths_from_loaded_tag_views(&mut self, paths: &[PathBuf]) {
        let removed_path_keys: FxHashSet<String> =
            paths.iter().map(|path| normalize_path_text(path)).collect();

        if tag_id_from_view_path(&self.navigation_state.current_path).is_some() {
            Self::retain_tag_view_items(&mut self.all_items, &removed_path_keys);
            Self::retain_tag_view_items(&mut self.items, &removed_path_keys);
            self.total_items = self.items.len();
        }

        if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
            if tag_id_from_view_path(&snapshot.path).is_some() {
                Self::retain_tag_view_items(&mut snapshot.all_items, &removed_path_keys);
                Self::retain_tag_view_items(&mut snapshot.items, &removed_path_keys);
                snapshot.total_items = snapshot.items.len();
            }
        }

        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter_mut().enumerate() {
            if index == active_tab {
                continue;
            }
            if tag_id_from_view_path(&tab.path).is_some() {
                Self::retain_tag_view_items(&mut tab.all_items, &removed_path_keys);
                Self::retain_tag_view_items(&mut tab.items, &removed_path_keys);
                tab.total_items = if tab.items_snapshot_compact {
                    tab.all_items.len()
                } else {
                    tab.items.len()
                };
            }

            if let Some(snapshot) = tab.dual_panel_inactive_state.as_mut() {
                if tag_id_from_view_path(&snapshot.path).is_some() {
                    Self::retain_tag_view_items(&mut snapshot.all_items, &removed_path_keys);
                    Self::retain_tag_view_items(&mut snapshot.items, &removed_path_keys);
                    snapshot.total_items = if snapshot.items_snapshot_compact {
                        snapshot.all_items.len()
                    } else {
                        snapshot.items.len()
                    };
                }
            }
        }
    }

    pub(crate) fn hide_unavailable_paths_from_tag_views(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        self.prune_paths_from_loaded_tag_views(paths);
        self.ui_ctx.request_repaint();
    }

    pub(crate) fn apply_ready_tag_view_hides(&mut self) {
        const MAX_REVALIDATIONS_PER_FRAME: usize = 32;

        let mut generations: FxHashMap<usize, bool> = FxHashMap::default();
        generations.insert(self.generation, !self.is_loading_folder);
        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            generations
                .entry(snapshot.generation)
                .and_modify(|ready| *ready |= !snapshot.is_loading_folder)
                .or_insert(!snapshot.is_loading_folder);
        }
        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter().enumerate() {
            if index != active_tab {
                generations
                    .entry(tab.generation)
                    .and_modify(|ready| *ready = true)
                    .or_insert(true);
            }
            if let Some(snapshot) = tab.dual_panel_inactive_state.as_ref() {
                generations
                    .entry(snapshot.generation)
                    .and_modify(|ready| *ready |= !snapshot.is_loading_folder)
                    .or_insert(!snapshot.is_loading_folder);
            }
        }

        self.pending_tag_view_hides
            .retain(|generation, paths| generations.contains_key(generation) && !paths.is_empty());

        let mut candidates = Vec::new();
        for (generation, ready) in &generations {
            if !ready || candidates.len() >= MAX_REVALIDATIONS_PER_FRAME {
                continue;
            }
            let Some(paths) = self.pending_tag_view_hides.get_mut(generation) else {
                continue;
            };
            while candidates.len() < MAX_REVALIDATIONS_PER_FRAME {
                let Some(path) = paths.pop() else {
                    break;
                };
                candidates.push(path);
            }
        }
        self.pending_tag_view_hides
            .retain(|_, paths| !paths.is_empty());

        let mut current_roots = crate::infrastructure::windows::RootAvailabilityCache::default();
        candidates.retain(|path| {
            !current_roots.is_root_accessible(path)
                || !crate::infrastructure::onedrive::fast_path_exists(path)
        });
        self.hide_unavailable_paths_from_tag_views(&candidates);

        if self
            .pending_tag_view_hides
            .keys()
            .any(|generation| generations.get(generation).copied().unwrap_or(false))
        {
            self.ui_ctx.request_repaint();
        }
    }
}
