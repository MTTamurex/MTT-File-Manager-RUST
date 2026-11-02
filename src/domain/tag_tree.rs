use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;

use super::file_tag::FileTag;

/// Tag IDs grouped by parent (`None` = root). Each sibling list is ordered by
/// `(position, name case-insensitive)`.
pub type TagChildrenMap = FxHashMap<Option<i64>, Vec<i64>>;

fn sibling_key(tag: &FileTag) -> (i64, String) {
    (tag.position, tag.name.to_lowercase())
}

fn sibling_sort_key(defs: &FxHashMap<i64, FileTag>, id: &i64) -> (i64, String) {
    defs.get(id)
        .map(sibling_key)
        .unwrap_or((i64::MAX, String::new()))
}

/// Groups tag IDs under their parent, siblings ordered canonically.
pub fn build_children_map(defs: &FxHashMap<i64, FileTag>) -> TagChildrenMap {
    let mut map: TagChildrenMap = FxHashMap::default();
    for tag in defs.values() {
        map.entry(tag.parent_id).or_default().push(tag.id);
    }
    for children in map.values_mut() {
        children.sort_by_key(|id| sibling_sort_key(defs, id));
    }
    map
}

/// Returns `root_id` plus every descendant ID, breadth-first.
/// The visited set keeps this safe even on malformed parent cycles.
pub fn subtree_ids(defs: &FxHashMap<i64, FileTag>, root_id: i64) -> Vec<i64> {
    let children = build_children_map(defs);
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    let mut visited: FxHashSet<i64> = FxHashSet::default();
    queue.push_back(root_id);
    visited.insert(root_id);
    while let Some(id) = queue.pop_front() {
        out.push(id);
        if let Some(children) = children.get(&Some(id)) {
            for child in children {
                if visited.insert(*child) {
                    queue.push_back(*child);
                }
            }
        }
    }
    out
}

/// True when giving `tag_id` the parent `new_parent` would create a cycle
/// (the new parent is the tag itself or one of its descendants).
pub fn would_create_cycle(defs: &FxHashMap<i64, FileTag>, tag_id: i64, new_parent: i64) -> bool {
    tag_id == new_parent || subtree_ids(defs, tag_id).contains(&new_parent)
}

/// Number of ancestors of `tag_id` (root = 0). Defensive against cycles.
pub fn tag_depth(defs: &FxHashMap<i64, FileTag>, tag_id: i64) -> usize {
    let mut depth = 0usize;
    let mut visited: FxHashSet<i64> = FxHashSet::default();
    let mut current = defs.get(&tag_id).and_then(|tag| tag.parent_id);
    while let Some(parent) = current {
        if !visited.insert(parent) {
            break;
        }
        depth += 1;
        current = defs.get(&parent).and_then(|tag| tag.parent_id);
    }
    depth
}

/// `tag_id` plus every ancestor up to the root. Defensive against cycles.
fn ancestor_chain(defs: &FxHashMap<i64, FileTag>, tag_id: i64) -> Vec<i64> {
    let mut chain = Vec::new();
    let mut visited: FxHashSet<i64> = FxHashSet::default();
    let mut current = Some(tag_id);
    while let Some(id) = current {
        if !visited.insert(id) {
            break;
        }
        chain.push(id);
        current = defs.get(&id).and_then(|tag| tag.parent_id);
    }
    chain
}

/// Distinct-path item count per tag **including all descendants**: for each
/// assigned path, every tag on its ancestor chain (self included) counts the
/// path once. Complements the per-tag counts derived from the same map.
pub fn compute_subtree_counts(
    defs: &FxHashMap<i64, FileTag>,
    assignments: &FxHashMap<PathBuf, Vec<i64>>,
) -> FxHashMap<i64, usize> {
    let mut counts: FxHashMap<i64, usize> = FxHashMap::default();
    for ids in assignments.values() {
        let mut touched: FxHashSet<i64> = FxHashSet::default();
        for id in ids {
            if !defs.contains_key(id) {
                continue;
            }
            for ancestor in ancestor_chain(defs, *id) {
                touched.insert(ancestor);
            }
        }
        for id in touched {
            *counts.entry(id).or_insert(0) += 1;
        }
    }
    counts
}

/// Normalizes a freshly loaded definition set in place: parents that no
/// longer exist become roots and parent cycles are broken at the starting tag.
pub fn repair_orphan_parents(tags: &mut [FileTag]) {
    let valid: FxHashSet<i64> = tags.iter().map(|tag| tag.id).collect();
    for tag in tags.iter_mut() {
        if let Some(parent) = tag.parent_id {
            if !valid.contains(&parent) {
                tag.parent_id = None;
            }
        }
    }

    for index in 0..tags.len() {
        let mut visited: FxHashSet<i64> = FxHashSet::default();
        visited.insert(tags[index].id);
        let mut current = tags[index].parent_id;
        let mut cyclic = false;
        while let Some(next) = current {
            if !visited.insert(next) {
                cyclic = true;
                break;
            }
            current = tags
                .iter()
                .find(|tag| tag.id == next)
                .and_then(|tag| tag.parent_id);
        }
        if cyclic {
            tags[index].parent_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_tag::TagColor;

    fn tag(id: i64, name: &str, position: i64, parent_id: Option<i64>) -> FileTag {
        FileTag {
            id,
            name: name.to_string(),
            color: TagColor::Blue,
            position,
            parent_id,
        }
    }

    fn defs(entries: Vec<FileTag>) -> FxHashMap<i64, FileTag> {
        entries.into_iter().map(|tag| (tag.id, tag)).collect()
    }

    #[test]
    fn children_map_orders_siblings_by_position_then_name() {
        let defs = defs(vec![
            tag(1, "Fotos", 0, None),
            tag(2, "zeta", 1, Some(1)),
            tag(3, "Alpha", 1, Some(1)),
            tag(4, "Músicas", 2, None),
        ]);
        let map = build_children_map(&defs);
        assert_eq!(map.get(&None), Some(&vec![1, 4]));
        assert_eq!(map.get(&Some(1)), Some(&vec![3, 2]));
    }

    #[test]
    fn subtree_ids_walks_all_descendants_breadth_first() {
        let defs = defs(vec![
            tag(1, "Fotos", 0, None),
            tag(2, "Férias", 1, Some(1)),
            tag(3, "2025", 0, Some(2)),
            tag(4, "Músicas", 2, None),
        ]);
        assert_eq!(subtree_ids(&defs, 1), vec![1, 2, 3]);
        assert_eq!(subtree_ids(&defs, 2), vec![2, 3]);
        assert_eq!(subtree_ids(&defs, 4), vec![4]);
    }

    #[test]
    fn subtree_ids_tolerates_parent_cycles() {
        let defs = defs(vec![tag(1, "a", 0, Some(2)), tag(2, "b", 1, Some(1))]);
        let ids = subtree_ids(&defs, 1);
        assert!(ids.contains(&1) && ids.contains(&2));
    }

    #[test]
    fn cycle_detection_rejects_self_and_descendants() {
        let defs = defs(vec![
            tag(1, "Fotos", 0, None),
            tag(2, "Férias", 1, Some(1)),
            tag(3, "Músicas", 2, None),
        ]);
        assert!(would_create_cycle(&defs, 1, 1));
        assert!(would_create_cycle(&defs, 1, 2));
        assert!(!would_create_cycle(&defs, 2, 1));
        assert!(!would_create_cycle(&defs, 2, 3));
    }

    #[test]
    fn depth_counts_ancestors() {
        let defs = defs(vec![
            tag(1, "Fotos", 0, None),
            tag(2, "Férias", 1, Some(1)),
            tag(3, "2025", 0, Some(2)),
        ]);
        assert_eq!(tag_depth(&defs, 1), 0);
        assert_eq!(tag_depth(&defs, 2), 1);
        assert_eq!(tag_depth(&defs, 3), 2);
    }

    #[test]
    fn subtree_counts_deduplicate_paths_per_root() {
        let defs = defs(vec![tag(1, "Fotos", 0, None), tag(2, "Férias", 1, Some(1))]);
        let mut assignments: FxHashMap<PathBuf, Vec<i64>> = FxHashMap::default();
        assignments.insert(PathBuf::from("C:\\media\\a.jpg"), vec![1, 2]);
        assignments.insert(PathBuf::from("C:\\media\\b.jpg"), vec![2]);
        assignments.insert(PathBuf::from("C:\\media\\c.txt"), vec![99]);

        let counts = compute_subtree_counts(&defs, &assignments);
        // "C:\media\a.jpg" and "C:\media\b.jpg" are inside the "Fotos" subtree;
        // only "a.jpg" carries the parent tag itself.
        assert_eq!(counts.get(&1), Some(&2));
        assert_eq!(counts.get(&2), Some(&2));
        assert!(!counts.contains_key(&99));
    }

    #[test]
    fn repair_fixes_orphans_and_breaks_cycles() {
        let mut tags = vec![
            tag(1, "a", 0, Some(42)),
            tag(2, "b", 1, Some(3)),
            tag(3, "c", 2, Some(2)),
        ];
        repair_orphan_parents(&mut tags);
        assert_eq!(tags[0].parent_id, None);
        // The 2 <-> 3 cycle is broken at the starting tag of the walk.
        let broken = tags.iter().filter(|t| t.parent_id.is_none()).count();
        assert!(broken >= 2);
        let defs = defs(tags);
        let children = build_children_map(&defs);
        assert!(children.contains_key(&None));
    }
}
