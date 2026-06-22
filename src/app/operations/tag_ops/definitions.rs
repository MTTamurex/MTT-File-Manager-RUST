use crate::app::global_search_state::GlobalSearchTagFilter;
use crate::app::state::ImageViewerApp;
use crate::domain::file_tag::{FileTag, TagColor};
use crate::domain::special_paths::{tag_id_from_view_path, tag_view_path, COMPUTER_VIEW_ID};
use rustc_hash::FxHashSet;
use std::sync::Arc;

impl ImageViewerApp {
    pub fn create_new_tag(&mut self, name: &str, color: TagColor) -> Option<i64> {
        let id = self.app_state_db.create_tag(name, color)?;
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
            },
        );
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

    pub fn delete_tag_definition(&mut self, tag_id: i64) -> bool {
        if !self.app_state_db.delete_tag(tag_id) {
            return false;
        }

        self.tag_definitions.remove(&tag_id);
        self.tag_counts.remove(&tag_id);
        let assignments = Arc::make_mut(&mut self.tag_assignments);
        assignments.retain(|_, ids| {
            ids.retain(|id| *id != tag_id);
            !ids.is_empty()
        });

        let mut changed_tags = FxHashSet::default();
        changed_tags.insert(tag_id);
        self.invalidate_cached_tag_views_for_tags(&changed_tags);

        if self.active_tag_filter == Some(tag_id) {
            self.active_tag_filter = None;
            self.save_preferences();
            if tag_id_from_view_path(&self.navigation_state.current_path) == Some(tag_id) {
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
        self.active_tag_filter = None;
        self.save_preferences();

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
            if fallback == COMPUTER_VIEW_ID {
                self.navigate_to_computer();
            } else if fallback == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
                self.navigate_to_recycle_bin();
            } else {
                self.navigate_to(&fallback);
            }
        } else {
            self.refresh_visible_items_after_tag_change();
        }
    }
}
