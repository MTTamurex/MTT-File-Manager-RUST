//! Incremental assignment mirrors/counts and coalesced view notifications.
use super::*;
use crate::app::state::ImageViewerApp;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct TagUpdates {
    pub changed: FxHashSet<i64>,
    pub next_flush: Option<Instant>,
    pub reapply_paths: FxHashSet<PathBuf>,
}

fn adjust(counts: &mut FxHashMap<i64, usize>, ids: impl Iterator<Item = i64>, add: bool) {
    for id in ids {
        if add {
            *counts.entry(id).or_default() += 1;
        } else if let Some(count) = counts.get_mut(&id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&id);
            }
        }
    }
}

fn expanded(
    ids: &[i64],
    defs: &FxHashMap<i64, FileTag>,
    chains: &mut FxHashMap<i64, Vec<i64>>,
) -> FxHashSet<i64> {
    ids.iter()
        .filter(|id| defs.contains_key(id))
        .flat_map(|id| {
            chains
                .entry(*id)
                .or_insert_with(|| crate::domain::tag_tree::ancestor_chain(defs, *id))
                .clone()
        })
        .collect()
}

struct Mirrors<'a> {
    tag_assignments: &'a mut Arc<FxHashMap<PathBuf, Vec<i64>>>,
    tag_assignments_normalized: &'a mut Arc<FxHashMap<String, Vec<i64>>>,
    tag_path_index: &'a mut super::path_index::TagPathIndex,
    tag_definitions: &'a FxHashMap<i64, FileTag>,
    tag_counts: &'a mut FxHashMap<i64, usize>,
    subtree_tag_counts: &'a mut FxHashMap<i64, usize>,
    changed: &'a mut FxHashSet<i64>,
}

impl Mirrors<'_> {
    fn apply(&mut self, changes: Vec<(PathBuf, Vec<i64>)>) -> bool {
        let changes: Vec<_> = changes
            .into_iter()
            .filter(|(path, ids)| {
                self.tag_assignments
                    .get(path)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    != ids.as_slice()
            })
            .collect();
        if changes.is_empty() {
            return false;
        }
        let mut keys = FxHashSet::default();
        let mut chains = FxHashMap::default();
        let assignments = Arc::make_mut(self.tag_assignments);
        for (path, ids) in changes {
            let old = assignments.get(&path).cloned().unwrap_or_default();
            if old == ids {
                continue;
            }
            let old_subtree = expanded(&old, self.tag_definitions, &mut chains);
            let new_subtree = expanded(&ids, self.tag_definitions, &mut chains);
            self.changed.extend(old.iter().chain(&ids).copied());
            self.changed
                .extend(old_subtree.iter().chain(&new_subtree).copied());
            adjust(self.tag_counts, old.into_iter(), false);
            adjust(self.tag_counts, ids.iter().copied(), true);
            adjust(self.subtree_tag_counts, old_subtree.into_iter(), false);
            adjust(self.subtree_tag_counts, new_subtree.into_iter(), true);
            keys.insert(normalize_path_text(&path));
            if ids.is_empty() {
                assignments.remove(&path);
                self.tag_path_index.remove(&path);
            } else {
                self.tag_path_index.insert(path.clone());
                assignments.insert(path, ids);
            }
        }
        if keys.is_empty() {
            return false;
        }
        let normalized = Arc::make_mut(self.tag_assignments_normalized);
        for key in keys {
            let mut ids = Vec::new();
            for alias in self.tag_path_index.exact(Path::new(&key)) {
                if let Some(assigned) = assignments.get(alias) {
                    for id in assigned {
                        if !ids.contains(id) {
                            ids.push(*id);
                        }
                    }
                }
            }
            if ids.is_empty() {
                normalized.remove(&key);
            } else {
                normalized.insert(key, ids);
            }
        }
        true
    }
}

impl ImageViewerApp {
    /// Update touched paths only; aliases merge for rendering and counts remain
    /// distinct per original path, exactly as in a complete rebuild.
    pub(super) fn replace_tag_paths(&mut self, changes: Vec<(PathBuf, Vec<i64>)>) {
        if (Mirrors {
            tag_assignments: &mut self.tag_assignments,
            tag_assignments_normalized: &mut self.tag_assignments_normalized,
            tag_path_index: &mut self.tag_path_index,
            tag_definitions: &self.tag_definitions,
            tag_counts: &mut self.tag_counts,
            subtree_tag_counts: &mut self.subtree_tag_counts,
            changed: &mut self.tag_updates.changed,
        })
        .apply(changes)
        {
            self.ui_ctx.request_repaint();
        }
    }

    pub(crate) fn flush_tag_updates(&mut self) {
        self.poll_tag_writes();
        if !self.tag_persistence.is_pending() && !self.tag_updates.reapply_paths.is_empty() {
            let paths = std::mem::take(&mut self.tag_updates.reapply_paths)
                .into_iter()
                .collect();
            self.auto_tag_manager.reapply_paths(paths);
        }
        if self.tag_updates.changed.is_empty() {
            return;
        }
        let now = Instant::now();
        if self.tag_persistence.is_pending()
            || self.tag_updates.next_flush.is_some_and(|due| now < due)
        {
            self.ui_ctx
                .request_repaint_after(Duration::from_millis(100));
            return;
        }
        let changed = std::mem::take(&mut self.tag_updates.changed);
        self.tag_updates.next_flush = Some(now + Duration::from_millis(100));
        self.mark_tag_assignments_changed();
        self.invalidate_cached_tag_views_for_tags(&changed);
        self.refresh_visible_items_for_changed_tags(&changed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_tag::TagColor;

    #[derive(Default)]
    struct Fixture {
        assignments: Arc<FxHashMap<PathBuf, Vec<i64>>>,
        normalized: Arc<FxHashMap<String, Vec<i64>>>,
        index: super::super::path_index::TagPathIndex,
        counts: FxHashMap<i64, usize>,
        subtree: FxHashMap<i64, usize>,
        changed: FxHashSet<i64>,
        defs: FxHashMap<i64, FileTag>,
    }

    impl Fixture {
        fn new() -> Self {
            let defs = [(1, None), (2, Some(1)), (3, Some(1))]
                .into_iter()
                .map(|(id, parent_id)| {
                    (
                        id,
                        FileTag {
                            id,
                            parent_id,
                            name: format!("Tag {id}"),
                            color: TagColor::Blue,
                            position: id,
                        },
                    )
                })
                .collect();
            Self {
                defs,
                ..Default::default()
            }
        }

        fn apply(&mut self, changes: &[(&str, &[i64])]) -> bool {
            Mirrors {
                tag_assignments: &mut self.assignments,
                tag_assignments_normalized: &mut self.normalized,
                tag_path_index: &mut self.index,
                tag_definitions: &self.defs,
                tag_counts: &mut self.counts,
                subtree_tag_counts: &mut self.subtree,
                changed: &mut self.changed,
            }
            .apply(
                changes
                    .iter()
                    .map(|(path, ids)| (PathBuf::from(path), ids.to_vec()))
                    .collect(),
            )
        }

        fn assert_rebuild_matches(&self) {
            let mut counts = FxHashMap::default();
            for ids in self.assignments.values() {
                for id in ids {
                    *counts.entry(*id).or_insert(0) += 1;
                }
            }
            assert_eq!(self.counts, counts);
            assert_eq!(
                self.subtree,
                crate::domain::tag_tree::compute_subtree_counts(&self.defs, &self.assignments)
            );
            let mut expected =
                super::super::normalized::build_tag_assignments_normalized(&self.assignments);
            let mut actual = self.normalized.as_ref().clone();
            for ids in expected.values_mut().chain(actual.values_mut()) {
                ids.sort_unstable();
            }
            assert_eq!(actual, expected);
            for path in self.assignments.keys() {
                assert!(self.index.exact(path).any(|key| key == path));
            }
        }
    }

    #[test]
    fn incremental_counts_and_alias_merges_match_full_rebuild_after_each_batch() {
        let mut fixture = Fixture::new();
        for changes in [
            vec![
                (r"C:\Data\file.txt", &[1, 2][..]),
                (r"C:\Other\file.txt", &[1, 3][..]),
            ],
            vec![(r"c:\data\FILE.txt", &[1, 3][..])],
            vec![(r"C:\Data\file.txt", &[1][..])],
            vec![(r"c:\data\FILE.txt", &[][..])],
            vec![
                (r"C:\Data\file.txt", &[][..]),
                (r"D:\Target\file.txt", &[1, 2, 3][..]),
            ],
            vec![(r"D:\Target\file.txt", &[][..])],
        ] {
            fixture.apply(&changes);
            fixture.assert_rebuild_matches();
        }
        assert!(fixture
            .index
            .canonical(Path::new(r"C:\Data\file.txt"))
            .is_none());
        assert_eq!(fixture.subtree.get(&1), Some(&1));
    }

    #[test]
    fn no_op_does_not_clone_shared_maps_and_changes_preserve_worker_snapshots() {
        let mut fixture = Fixture::new();
        fixture.apply(&[(r"C:\Data\file.txt", &[1, 2])]);
        let old = fixture.assignments.clone();
        let normalized = fixture.normalized.clone();
        assert!(!fixture.apply(&[(r"C:\Data\file.txt", &[1, 2]), (r"D:\Missing", &[])]));
        assert!(Arc::ptr_eq(&old, &fixture.assignments));
        assert!(Arc::ptr_eq(&normalized, &fixture.normalized));
        fixture.apply(&[(r"C:\Data\file.txt", &[])]);
        assert_eq!(old.len(), 1);
        assert_eq!(normalized.len(), 1);
        assert!(fixture.assignments.is_empty());
        fixture.assert_rebuild_matches();
    }
}
