use crate::app::global_search_state::GlobalSearchTagFilter;
use crate::app::state::ImageViewerApp;
use crate::domain::file_tag::{FileTag, TagColor};
use crate::domain::special_paths::{tag_id_from_view_path, tag_view_path, COMPUTER_VIEW_ID};
use crate::domain::tag_tree;
use rustc_hash::FxHashSet;
use std::path::PathBuf;
use std::sync::Arc;

/// A pending "uncheck parent" action that would remove sub-tags from files.
/// Held until the user confirms in the cascade modal.
#[derive(Debug, Clone)]
pub struct PendingTagCascade {
    pub paths: Vec<PathBuf>,
    pub tag_id: i64,
    pub tag_name: String,
    /// Number of sub-tag assignments that confirming will remove.
    pub sub_tag_assignments: usize,
}

impl ImageViewerApp {
    /// Entry point for the context menu's tag toggle. Unchecking a tag that has
    /// sub-tags on the target files opens a confirmation first; everything
    /// else is applied directly.
    pub fn request_tag_toggle(&mut self, paths: &[PathBuf], tag_id: i64) {
        let cascade = if self.paths_have_tag(paths, tag_id) {
            self.cascade_count_for_unassign(paths, tag_id)
        } else {
            0
        };
        if cascade > 0 {
            let tag_name = self
                .tag_definitions
                .get(&tag_id)
                .map(|tag| tag.name.clone())
                .unwrap_or_default();
            self.pending_tag_cascade = Some(PendingTagCascade {
                paths: paths.to_vec(),
                tag_id,
                tag_name,
                sub_tag_assignments: cascade,
            });
            self.ui_ctx.request_repaint();
        } else {
            self.toggle_tag_on_paths(paths, tag_id);
        }
    }

    /// Applies the pending cascade after the user confirmed it.
    pub fn confirm_pending_tag_cascade(&mut self) {
        if let Some(pending) = self.pending_tag_cascade.take() {
            self.unassign_tag_from_paths(&pending.paths, pending.tag_id);
        }
    }

    pub fn cancel_pending_tag_cascade(&mut self) {
        self.pending_tag_cascade = None;
    }

    pub fn create_new_tag(
        &mut self,
        name: &str,
        color: TagColor,
        parent_id: Option<i64>,
    ) -> Option<i64> {
        if let Some(parent) = parent_id {
            if !self.tag_definitions.contains_key(&parent) {
                return None;
            }
        }
        let id = self.app_state_db.create_tag(name, color, parent_id)?;
        let position = self
            .tag_definitions
            .values()
            .map(|tag| tag.position)
            .max()
            .unwrap_or(-1)
            + 1;
        self.tag_definitions.insert(
            id,
            FileTag {
                id,
                name: name.trim().to_string(),
                color,
                position,
                parent_id,
            },
        );
        self.rebuild_sorted_tag_ids();
        self.sync_sidebar_tag_order();
        self.ui_ctx.request_repaint();
        Some(id)
    }

    pub fn rename_tag_definition(&mut self, tag_id: i64, name: &str) -> bool {
        if !self.app_state_db.rename_tag(tag_id, name) {
            return false;
        }
        if let Some(tag) = self.tag_definitions.get_mut(&tag_id) {
            tag.name = name.trim().to_string();
        }
        // Name participates in the sort key, so the snapshot must be rebuilt.
        self.rebuild_sorted_tag_ids();
        self.ui_ctx.request_repaint();
        true
    }

    pub fn update_tag_definition_color(&mut self, tag_id: i64, color: TagColor) -> bool {
        if !self.app_state_db.update_tag_color(tag_id, color) {
            return false;
        }
        if let Some(tag) = self.tag_definitions.get_mut(&tag_id) {
            tag.color = color;
        }
        self.ui_ctx.request_repaint();
        true
    }

    /// Moves a tag under another parent (`None` = root). Rejects invalid
    /// parents: unknown tags, the tag itself, and any of its descendants.
    pub fn set_tag_parent(&mut self, tag_id: i64, parent_id: Option<i64>) -> bool {
        if let Some(parent) = parent_id {
            if !self.tag_definitions.contains_key(&parent)
                || tag_tree::would_create_cycle(&self.tag_definitions, tag_id, parent)
            {
                return false;
            }
        }
        let old_parent = self
            .tag_definitions
            .get(&tag_id)
            .and_then(|tag| tag.parent_id);
        if old_parent == parent_id || !self.app_state_db.update_tag_parent(tag_id, parent_id) {
            return false;
        }
        if let Some(tag) = self.tag_definitions.get_mut(&tag_id) {
            tag.parent_id = parent_id;
        }

        // The union each view displays changed for the moved subtree and for
        // both ancestry chains, so cached tag listings are stale.
        self.rebuild_sorted_tag_ids();
        self.sync_sidebar_tag_order();
        // Subtree counts follow the hierarchy, not the assignments.
        self.recompute_tag_counts_from_assignments();
        let mut affected = FxHashSet::default();
        for chain_start in [Some(tag_id), old_parent, parent_id] {
            let Some(start) = chain_start else {
                continue;
            };
            affected.extend(tag_tree::subtree_ids(&self.tag_definitions, start));
            let mut current = self.tag_definitions.get(&start).and_then(|t| t.parent_id);
            while let Some(id) = current {
                if !affected.insert(id) {
                    break;
                }
                current = self.tag_definitions.get(&id).and_then(|t| t.parent_id);
            }
        }
        self.invalidate_cached_tag_views_for_tags(&affected);
        self.reload_visible_tag_views();
        self.ui_ctx.request_repaint();
        true
    }

    /// Deletes a tag together with its whole subtree and every assignment
    /// pointing at any of them. Tag metadata only — files and folders on
    /// disk are never touched.
    pub fn delete_tag_definition(&mut self, tag_id: i64) -> bool {
        if !self.tag_definitions.contains_key(&tag_id) {
            return false;
        }
        let removed_ids = tag_tree::subtree_ids(&self.tag_definitions, tag_id);
        let removed: FxHashSet<i64> = removed_ids.iter().copied().collect();
        let removed_tags = self.app_state_db.delete_tag_subtree(&removed_ids);
        if removed_tags == 0 {
            return false;
        }

        for id in &removed_ids {
            self.tag_definitions.remove(id);
            self.sidebar_tags_collapsed.remove(id);
            self.tag_manager_edit_names.remove(id);
        }
        self.rebuild_sorted_tag_ids();
        self.sync_sidebar_tag_order();
        let assignments = Arc::make_mut(&mut self.tag_assignments);
        assignments.retain(|_, ids| {
            ids.retain(|id| !removed.contains(id));
            !ids.is_empty()
        });
        self.recompute_tag_counts_from_assignments();

        self.invalidate_cached_tag_views_for_tags(&removed);

        if self
            .active_tag_filter
            .is_some_and(|active| removed.contains(&active))
        {
            self.active_tag_filter = None;
            self.save_preferences();
            if tag_id_from_view_path(&self.navigation_state.current_path)
                .is_some_and(|id| removed.contains(&id))
            {
                self.navigate_to_computer();
            }
        }

        // Drop any stale tag IDs from the active global search filter.
        self.prune_stale_tag_filter();

        self.sync_tag_assignments_normalized();
        self.refresh_visible_items_after_tag_change();
        true
    }

    /// Remove any tag IDs from the global search tag filter that no longer
    /// have a corresponding `FileTag` definition. If the filter was in the
    /// `Selected` state and the pruned list becomes empty, the filter is
    /// reset to `All` (no tag filter applied).
    pub fn prune_stale_tag_filter(&mut self) {
        if let GlobalSearchTagFilter::Selected(ids) = &mut self.global_search.tag_filter {
            ids.retain(|id| self.tag_definitions.contains_key(id));
            if ids.is_empty() {
                self.global_search.tag_filter = GlobalSearchTagFilter::All;
            }
        }
    }

    pub fn set_tag_filter(&mut self, tag_id: Option<i64>) {
        let tag_id = tag_id.filter(|id| self.tag_definitions.contains_key(id));
        if let Some(tag_id) = tag_id {
            let view_path = tag_view_path(tag_id);
            if self.redirect_pinned_navigation(&view_path) {
                return;
            }
            if self.navigation_state.current_path != view_path {
                self.navigation_state.navigation.navigate_to(view_path);
            }
            self.setup_tag_view(tag_id);
            self.save_preferences();
            self.sync_to_tab();
            return;
        }

        let in_tag_view = tag_id_from_view_path(&self.navigation_state.current_path).is_some();
        if self.active_tag_filter.is_none() && !in_tag_view {
            return;
        }
        if in_tag_view {
            let fallback = self
                .navigation_state
                .navigation
                .paths
                .iter()
                .take(self.navigation_state.navigation.current_index)
                .rev()
                .find(|path| tag_id_from_view_path(path).is_none())
                .cloned()
                .unwrap_or_else(|| COMPUTER_VIEW_ID.to_string());
            if self.redirect_pinned_navigation(&fallback) {
                return;
            }
            self.active_tag_filter = None;
            self.save_preferences();
            if fallback == COMPUTER_VIEW_ID {
                self.navigate_to_computer();
            } else if fallback == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
                self.navigate_to_recycle_bin();
            } else {
                self.navigate_to(&fallback);
            }
        } else {
            self.active_tag_filter = None;
            self.save_preferences();
            self.refresh_visible_items_after_tag_change();
        }
    }
}
