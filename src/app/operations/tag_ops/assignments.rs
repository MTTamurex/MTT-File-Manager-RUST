//! Exact-path tag edits use the normalized index, including negative lookups.
use super::persistence::TagWrite;
use super::*;
use crate::app::state::ImageViewerApp;
use crate::domain::file_tag;

impl ImageViewerApp {
    pub fn assign_tag_to_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if paths.is_empty() || !self.tag_definitions.contains_key(&tag_id) {
            return;
        }
        let ids = crate::domain::tag_tree::tags_to_assign(&self.tag_definitions, tag_id);
        let mut seen = FxHashSet::default();
        let mut changes = Vec::new();
        for path in paths {
            if path.to_str().is_none()
                || !seen.insert(normalize_path_text(path))
                || file_tag::path_has_tag(&self.tag_assignments_normalized, path, tag_id)
            {
                continue;
            }
            let path = self
                .tag_path_index
                .canonical(path)
                .unwrap_or_else(|| path.clone());
            let mut assigned = self.tag_assignments.get(&path).cloned().unwrap_or_default();
            for id in &ids {
                if !assigned.contains(id) {
                    assigned.push(*id);
                }
            }
            changes.push((path, assigned));
        }
        if changes.is_empty() {
            return;
        }
        let paths: Vec<_> = changes.iter().map(|(path, _)| path.clone()).collect();
        let mut pending: FxHashSet<_> =
            paths.iter().map(|path| normalize_path_text(path)).collect();
        let mut entries = Vec::new();
        for entry in self.all_items.iter().chain(self.items.iter()) {
            if pending.remove(&normalize_path_text(&entry.path)) {
                entries.push(entry.clone());
                if pending.is_empty() {
                    break;
                }
            }
        }
        if self.queue_tag_write(TagWrite::Assign(paths, ids, entries)) {
            self.replace_tag_paths(changes);
        }
    }

    /// Remove the requested tag and descendants, preserving ancestors and aliases.
    pub fn unassign_tag_from_paths(&mut self, paths: &[PathBuf], tag_id: i64) -> usize {
        let ids = crate::domain::tag_tree::tags_to_remove(&self.tag_definitions, tag_id);
        let removable: FxHashSet<_> = ids.iter().copied().collect();
        let mut seen = FxHashSet::default();
        let mut changes = Vec::new();
        let mut cascaded = 0;
        for path in paths {
            for key in self.tag_path_index.exact(path) {
                if !seen.insert(key.clone()) {
                    continue;
                }
                let Some(assigned) = self.tag_assignments.get(key) else {
                    continue;
                };
                if !assigned.iter().any(|id| removable.contains(id)) {
                    continue;
                }
                cascaded += assigned
                    .iter()
                    .filter(|id| **id != tag_id && removable.contains(id))
                    .count();
                changes.push((
                    key.clone(),
                    assigned
                        .iter()
                        .filter(|id| !removable.contains(id))
                        .copied()
                        .collect(),
                ));
            }
        }
        if !changes.is_empty() {
            let paths = changes.iter().map(|(path, _)| path.clone()).collect();
            if self.queue_tag_write(TagWrite::Unassign(paths, ids)) {
                self.replace_tag_paths(changes);
            } else {
                return 0;
            }
        }
        cascaded
    }

    pub fn cascade_count_for_unassign(&self, paths: &[PathBuf], tag_id: i64) -> usize {
        let removable: FxHashSet<_> =
            crate::domain::tag_tree::tags_to_remove(&self.tag_definitions, tag_id)
                .into_iter()
                .filter(|id| *id != tag_id)
                .collect();
        if removable.is_empty() {
            return 0;
        }
        let mut seen = FxHashSet::default();
        paths
            .iter()
            .flat_map(|path| self.tag_path_index.exact(path))
            .filter(|key| seen.insert((*key).clone()))
            .filter_map(|key| self.tag_assignments.get(key))
            .map(|ids| ids.iter().filter(|id| removable.contains(id)).count())
            .sum()
    }

    pub fn toggle_tag_on_paths(&mut self, paths: &[PathBuf], tag_id: i64) {
        if self.paths_have_tag(paths, tag_id) {
            self.unassign_tag_from_paths(paths, tag_id);
        } else {
            self.assign_tag_to_paths(paths, tag_id);
        }
    }

    pub fn paths_have_tag(&self, paths: &[PathBuf], tag_id: i64) -> bool {
        !paths.is_empty()
            && paths
                .iter()
                .all(|path| file_tag::path_has_tag(&self.tag_assignments_normalized, path, tag_id))
    }

    pub fn paths_tag_ids(&self, paths: &[PathBuf]) -> Vec<i64> {
        let mut seen = FxHashSet::default();
        for path in paths {
            if let Some(ids) = file_tag::tag_ids_for_path(&self.tag_assignments_normalized, path) {
                seen.extend(ids.iter().copied());
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
}
