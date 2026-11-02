use super::*;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::domain::file_tag;
use rustc_hash::FxHashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

impl ImageViewerApp {
    /// Assigns `tag_id` to `paths`. Under the implicit-parent model the tag's
    /// ancestors are assigned too, so a file never carries a sub-tag without
    /// its parent.
    pub fn assign_tag_to_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if paths.is_empty() || !self.tag_definitions.contains_key(&tag_id) {
            return;
        }
        let tag_ids = crate::domain::tag_tree::tags_to_assign(&self.tag_definitions, tag_id);

        let mut seen_paths = FxHashSet::default();
        let mut paths_to_assign = Vec::new();
        for path in paths {
            if path.to_str().is_none() {
                continue;
            }
            // Only paths missing the requested tag need work; a path that
            // already has `tag_id` also has its ancestors by invariant.
            if file_tag::path_has_tag(self.tag_assignments_normalized.as_ref(), path, tag_id) {
                continue;
            }

            let assignment_path = tag_assignment_key_for_path(&self.tag_assignments, path)
                .cloned()
                .unwrap_or_else(|| path.clone());
            if seen_paths.insert(normalize_path_text(&assignment_path)) {
                paths_to_assign.push(assignment_path);
            }
        }

        if paths_to_assign.is_empty() {
            return;
        }
        if !self
            .app_state_db
            .assign_tags_batch(&paths_to_assign, &tag_ids)
        {
            return;
        }

        let entries_to_cache = {
            let mut pending_keys: FxHashSet<String> = paths_to_assign
                .iter()
                .map(|path| normalize_path_text(path))
                .collect();
            let mut entries = Vec::new();
            for entry in self.all_items.iter().chain(self.items.iter()) {
                let key = normalize_path_text(&entry.path);
                if pending_keys.remove(&key) {
                    entries.push(entry.clone());
                    if pending_keys.is_empty() {
                        break;
                    }
                }
            }
            entries
        };

        if !entries_to_cache.is_empty() {
            self.app_state_db
                .upsert_cached_file_entries(&entries_to_cache);
        }

        let mut changed = false;
        for path in paths_to_assign {
            let assignments = Arc::make_mut(&mut self.tag_assignments);
            let assignment_key = tag_assignment_key_for_path(assignments, &path)
                .cloned()
                .unwrap_or(path);
            let ids = assignments.entry(assignment_key).or_default();
            for id in &tag_ids {
                if !ids.contains(id) {
                    ids.push(*id);
                    changed = true;
                }
            }
        }

        if changed {
            // Subtree counts aggregate distinct paths per ancestor, so a
            // full recompute (not a +1) is required.
            self.recompute_tag_counts_from_assignments();
            self.sync_tag_assignments_normalized();
            let changed_tags: FxHashSet<i64> = tag_ids.into_iter().collect();
            self.invalidate_cached_tag_views_for_tags(&changed_tags);
            self.refresh_visible_items_after_tag_change();
        }
    }

    /// Removes `tag_id` and its sub-tags from `paths`. Ancestors are kept.
    /// Returns how many sub-tag assignments were removed in addition to the
    /// tag itself, so the caller can confirm the cascade with the user.
    pub fn unassign_tag_from_paths(&mut self, paths: &[PathBuf], tag_id: i64) -> usize {
        if paths.is_empty() {
            return 0;
        }
        let tag_ids = crate::domain::tag_tree::tags_to_remove(&self.tag_definitions, tag_id);
        let removable: FxHashSet<i64> = tag_ids.iter().copied().collect();

        let mut seen_paths = FxHashSet::default();
        let mut paths_to_unassign = Vec::new();
        for path in paths {
            if let Some(assignment_key) =
                tag_assignment_key_for_path_with_any(&self.tag_assignments, path, &removable)
            {
                if seen_paths.insert(normalize_path_text(assignment_key)) {
                    paths_to_unassign.push(assignment_key.clone());
                }
            }
        }

        if paths_to_unassign.is_empty()
            || !self
                .app_state_db
                .unassign_tags_batch(&paths_to_unassign, &tag_ids)
        {
            return 0;
        }

        let mut changed = false;
        let mut cascaded = 0usize;
        for path in paths_to_unassign {
            let assignments = Arc::make_mut(&mut self.tag_assignments);
            let assignment_key =
                tag_assignment_key_for_path_with_any(assignments, &path, &removable)
                    .cloned()
                    .unwrap_or(path);
            if let Some(ids) = assignments.get_mut(&assignment_key) {
                // Count only sub-tags (not the clicked tag itself) that were
                // actually present, so the confirmation matches what is removed.
                cascaded += ids
                    .iter()
                    .filter(|id| removable.contains(id) && **id != tag_id)
                    .count();
                let before_len = ids.len();
                ids.retain(|id| !removable.contains(id));
                if ids.len() != before_len {
                    changed = true;
                }
                if ids.is_empty() {
                    assignments.remove(&assignment_key);
                }
            }
        }

        if changed {
            self.recompute_tag_counts_from_assignments();
            self.sync_tag_assignments_normalized();
            self.invalidate_cached_tag_views_for_tags(&removable);
            self.refresh_visible_items_after_tag_change();
        }
        cascaded
    }

    /// Number of sub-tag assignments that unchecking `tag_id` would remove from
    /// `paths` in addition to the tag itself. Read-only: the UI uses it to
    /// confirm a cascade before calling [`Self::unassign_tag_from_paths`].
    pub fn cascade_count_for_unassign(&self, paths: &[PathBuf], tag_id: i64) -> usize {
        let removable: FxHashSet<i64> =
            crate::domain::tag_tree::tags_to_remove(&self.tag_definitions, tag_id)
                .into_iter()
                .filter(|id| *id != tag_id)
                .collect();
        if removable.is_empty() {
            return 0;
        }
        let mut seen = FxHashSet::default();
        let mut total = 0usize;
        for path in paths {
            if let Some(key) =
                tag_assignment_key_for_path_with_any(&self.tag_assignments, path, &removable)
            {
                if seen.insert(normalize_path_text(key)) {
                    total += self.tag_assignments[key]
                        .iter()
                        .filter(|id| removable.contains(id))
                        .count();
                }
            }
        }
        total
    }

    /// Unchecks a tag on `paths` when it is present, or checks it (with its
    /// ancestors) otherwise. Unchecking a parent also removes its sub-tags from
    /// the affected files; the menu confirms that cascade beforehand through
    /// [`Self::cascade_count_for_unassign`].
    pub fn toggle_tag_on_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if self.paths_have_tag(paths, tag_id) {
            self.unassign_tag_from_paths(paths, tag_id);
        } else {
            self.assign_tag_to_paths(paths, tag_id);
        }
    }

    pub fn paths_have_tag(&self, paths: &[PathBuf], tag_id: i64) -> bool {
        !paths.is_empty()
            && paths.iter().all(|path| {
                file_tag::path_has_tag(self.tag_assignments_normalized.as_ref(), path, tag_id)
            })
    }

    pub fn paths_tag_ids(&self, paths: &[PathBuf]) -> Vec<i64> {
        let mut seen = FxHashSet::default();
        for path in paths {
            if let Some(ids) =
                file_tag::tag_ids_for_path(self.tag_assignments_normalized.as_ref(), path)
            {
                for id in ids {
                    seen.insert(*id);
                }
            }
        }

        let mut ids: Vec<i64> = seen.into_iter().collect();
        ids.sort_by_key(|id| {
            self.tag_definitions
                .get(id)
                .map(tag_sort_key)
                .unwrap_or((i64::MAX, String::new()))
        });
        ids
    }

    pub fn clear_tag_assignments_for_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let affected_assignments: Vec<(PathBuf, Vec<i64>)> = self
            .tag_assignments
            .iter()
            .filter(|(assigned_path, _)| {
                paths
                    .iter()
                    .any(|path| path_is_same_or_descendant(assigned_path, path))
            })
            .map(|(assigned_path, ids)| (assigned_path.clone(), ids.clone()))
            .collect();
        if affected_assignments.is_empty() {
            return;
        }

        let changed_tags: FxHashSet<i64> = affected_assignments
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        let affected_paths: Vec<PathBuf> = affected_assignments
            .iter()
            .map(|(path, _)| path.clone())
            .collect();

        if self
            .app_state_db
            .clear_tag_assignments_for_paths(&affected_paths)
            .is_none()
        {
            return;
        }

        let assignments = Arc::make_mut(&mut self.tag_assignments);
        let before_len = assignments.len();
        assignments.retain(|assigned_path, _| {
            !paths
                .iter()
                .any(|path| path_is_same_or_descendant(assigned_path, path))
        });
        let changed = assignments.len() != before_len;
        if changed {
            self.recompute_tag_counts_from_assignments();
            self.sync_tag_assignments_normalized();
            self.invalidate_cached_tag_views_for_tags(&changed_tags);
            self.refresh_visible_items_after_tag_change();
        }
    }

    pub fn clear_tag_assignments_for_copied_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        let affected_assignments: Vec<(PathBuf, Vec<i64>)> = self
            .tag_assignments
            .iter()
            .filter(|(assigned_path, _)| {
                paths
                    .iter()
                    .any(|path| path_is_same_or_descendant(assigned_path, path))
            })
            .map(|(assigned_path, ids)| (assigned_path.clone(), ids.clone()))
            .collect();

        let changed_tags: FxHashSet<i64> = affected_assignments
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        let affected_paths: Vec<PathBuf> = if affected_assignments.is_empty() {
            paths.to_vec()
        } else {
            affected_assignments
                .iter()
                .map(|(path, _)| path.clone())
                .collect()
        };

        let Some(removed_from_db) = self
            .app_state_db
            .clear_tag_assignments_for_paths(&affected_paths)
        else {
            return;
        };

        let assignments = Arc::make_mut(&mut self.tag_assignments);
        let before_len = assignments.len();
        assignments.retain(|assigned_path, _| {
            !paths
                .iter()
                .any(|path| path_is_same_or_descendant(assigned_path, path))
        });
        let memory_changed = assignments.len() != before_len;

        if memory_changed {
            self.recompute_tag_counts_from_assignments();
            self.sync_tag_assignments_normalized();
            self.invalidate_cached_tag_views_for_tags(&changed_tags);
            self.refresh_visible_items_after_tag_change();
        } else if removed_from_db > 0 {
            self.set_tag_assignments(self.app_state_db.get_all_tag_assignments());
            self.recompute_tag_counts_from_assignments();
            self.reload_visible_tag_views();
            self.ui_ctx.request_repaint();
        }
    }

    pub fn move_tag_assignments_for_path(&mut self, old_path: &Path, new_path: &Path) {
        if old_path == new_path {
            return;
        }

        let moved_assignments: Vec<(PathBuf, PathBuf, Vec<i64>)> = self
            .tag_assignments
            .iter()
            .filter(|(path, _)| path_is_same_or_descendant(path, old_path))
            .map(|(path, ids)| {
                (
                    path.clone(),
                    remap_path(path, old_path, new_path),
                    ids.clone(),
                )
            })
            .collect();
        if moved_assignments.is_empty() {
            return;
        }

        let changed_tags: FxHashSet<i64> = moved_assignments
            .iter()
            .flat_map(|(_, _, ids)| ids.iter().copied())
            .collect();

        let db_moves: Vec<(PathBuf, PathBuf, i64)> = moved_assignments
            .iter()
            .flat_map(|(old_assigned_path, new_assigned_path, ids)| {
                ids.iter()
                    .map(|id| (old_assigned_path.clone(), new_assigned_path.clone(), *id))
            })
            .collect();

        if !self.app_state_db.move_tag_assignments(&db_moves) {
            return;
        }

        let remap_entry = |entry: &mut FileEntry| {
            if path_is_same_or_descendant(&entry.path, old_path) {
                entry.path = remap_path(&entry.path, old_path, new_path);
                if let Some(name) = entry.path.file_name() {
                    entry.name = name.to_string_lossy().to_string();
                }
            }
            if let Some(cover) = entry.folder_cover.as_mut() {
                if path_is_same_or_descendant(cover, old_path) {
                    *cover = remap_path(cover, old_path, new_path);
                }
            }
        };
        let items_diverged = !Arc::ptr_eq(&self.items, &self.all_items);
        for entry in Arc::make_mut(&mut self.all_items).iter_mut() {
            remap_entry(entry);
        }
        if items_diverged {
            for entry in Arc::make_mut(&mut self.items).iter_mut() {
                remap_entry(entry);
            }
        } else {
            self.share_visible_items_from_all_items();
        }

        if let Some(snapshot) = self.dual_panel_inactive_state.as_mut() {
            for entry in Arc::make_mut(&mut snapshot.all_items).iter_mut() {
                remap_entry(entry);
            }
            if !snapshot.items_snapshot_compact {
                for entry in Arc::make_mut(&mut snapshot.items).iter_mut() {
                    remap_entry(entry);
                }
            }
        }

        let assignments = Arc::make_mut(&mut self.tag_assignments);
        for (old_assigned_path, new_assigned_path, ids) in moved_assignments {
            assignments.remove(&old_assigned_path);
            let assignment_key = tag_assignment_key_for_path(assignments, &new_assigned_path)
                .cloned()
                .unwrap_or(new_assigned_path);
            let target = assignments.entry(assignment_key).or_default();
            for id in ids {
                if !target.contains(&id) {
                    target.push(id);
                }
            }
        }
        self.recompute_tag_counts_from_assignments();
        self.invalidate_cached_tag_views_for_tags(&changed_tags);
        self.sync_tag_assignments_normalized();
        self.refresh_visible_items_after_tag_change();
    }
}
