use crate::domain::file_tag::FileTag;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

mod assignments;
mod cache;
mod definitions;
pub(crate) mod normalized;
pub(crate) mod purge_worker;
mod sidebar_order;
mod view;
mod view_hides;
mod view_reload;

pub(crate) use sidebar_order::{
    parse_sidebar_tag_order, reconcile_sidebar_tag_order, serialize_sidebar_tag_order,
};

pub(crate) enum TagPathUpdate {
    PersistedRemoval(Vec<PathBuf>),
    HideFromViews {
        generation: usize,
        paths: Vec<PathBuf>,
    },
}

fn normalize_path_text(path: &Path) -> String {
    crate::domain::file_tag::normalize_tag_path_key(path)
}

fn path_is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    let candidate = normalize_path_text(candidate);
    let root = normalize_path_text(root);
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn remap_path(candidate: &Path, old_root: &Path, new_root: &Path) -> PathBuf {
    if let Ok(suffix) = candidate.strip_prefix(old_root) {
        return new_root.join(suffix);
    }

    let candidate = candidate.to_string_lossy();
    let old_root = old_root.to_string_lossy();
    let new_root = new_root.to_string_lossy();
    let old_root = old_root.trim_end_matches(['\\', '/']);
    if candidate.len() <= old_root.len() {
        return PathBuf::from(new_root.as_ref());
    }

    let suffix = &candidate[old_root.len()..];
    PathBuf::from(format!(
        "{}{}",
        new_root.trim_end_matches(['\\', '/']),
        suffix
    ))
}

fn tag_sort_key(tag: &FileTag) -> (i64, String) {
    (tag.position, tag.name.to_lowercase())
}

/// Builds the ordered tag-ID snapshot from the current definitions.
///
/// Depth-first pre-order: root tags in `(position, name case-insensitive)`
/// order, each followed by its own subtree. Consumers that need the tree
/// structure use [`crate::domain::tag_tree::build_children_map`].
pub(crate) fn build_sorted_tag_ids(defs: &FxHashMap<i64, FileTag>) -> std::sync::Arc<[i64]> {
    let children = crate::domain::tag_tree::build_children_map(defs);
    let mut out: Vec<i64> = Vec::with_capacity(defs.len());
    fn walk(
        children: &crate::domain::tag_tree::TagChildrenMap,
        parent: Option<i64>,
        out: &mut Vec<i64>,
    ) {
        if let Some(ids) = children.get(&parent) {
            for id in ids {
                out.push(*id);
                walk(children, Some(*id), out);
            }
        }
    }
    walk(&children, None, &mut out);

    // Defensive: malformed parent cycles would leave IDs unreachable from the
    // roots; append them in sibling order so nothing silently disappears.
    if out.len() != defs.len() {
        let mut leftover: Vec<&FileTag> =
            defs.values().filter(|tag| !out.contains(&tag.id)).collect();
        leftover.sort_by_key(|tag| tag_sort_key(tag));
        out.extend(leftover.iter().map(|tag| tag.id));
    }
    out.into()
}

fn tag_assignment_path_matches(assigned_path: &Path, path: &Path) -> bool {
    normalize_path_text(assigned_path) == normalize_path_text(path)
}

fn tag_assignment_key_for_path<'a>(
    assignments: &'a FxHashMap<PathBuf, Vec<i64>>,
    path: &Path,
) -> Option<&'a PathBuf> {
    if let Some((key, _)) = assignments.get_key_value(path) {
        return Some(key);
    }

    assignments
        .keys()
        .find(|assigned_path| tag_assignment_path_matches(assigned_path, path))
}

fn tag_assignment_key_for_path_with_tag<'a>(
    assignments: &'a FxHashMap<PathBuf, Vec<i64>>,
    path: &Path,
    tag_id: i64,
) -> Option<&'a PathBuf> {
    if let Some((key, ids)) = assignments.get_key_value(path) {
        if ids.contains(&tag_id) {
            return Some(key);
        }
    }

    assignments.iter().find_map(|(assigned_path, ids)| {
        (ids.contains(&tag_id) && tag_assignment_path_matches(assigned_path, path))
            .then_some(assigned_path)
    })
}

#[cfg(test)]
mod sorted_tag_ids_tests {
    use super::build_sorted_tag_ids;
    use crate::domain::file_tag::{FileTag, TagColor};
    use rustc_hash::FxHashMap;

    fn tag(id: i64, name: &str, position: i64) -> FileTag {
        FileTag {
            id,
            name: name.to_string(),
            color: TagColor::Red,
            position,
            parent_id: None,
        }
    }

    #[test]
    fn orders_by_position_then_name_case_insensitive() {
        let mut defs: FxHashMap<i64, FileTag> = FxHashMap::default();
        defs.insert(1, tag(1, "beta", 1));
        defs.insert(2, tag(2, "Alpha", 1)); // same position, case-insensitive before "beta"
        defs.insert(3, tag(3, "zeta", 0)); // lower position sorts first
        let ids = build_sorted_tag_ids(&defs);
        assert_eq!(&*ids, &[3, 2, 1]);
    }

    #[test]
    fn orders_depth_first_with_children_under_their_parent() {
        let mut defs: FxHashMap<i64, FileTag> = FxHashMap::default();
        defs.insert(1, tag(1, "Fotos", 0));
        defs.insert(2, tag(2, "Músicas", 1));
        let mut ferias = tag(3, "Férias", 0);
        ferias.parent_id = Some(1);
        defs.insert(3, ferias);
        let ids = build_sorted_tag_ids(&defs);
        assert_eq!(&*ids, &[1, 3, 2]);
    }
}
