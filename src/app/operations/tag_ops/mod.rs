use crate::domain::file_tag::FileTag;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

mod assignments;
mod auto_tag;
mod cache;
mod definitions;
pub(crate) mod migration;
mod moves;
pub(crate) mod normalized;
pub(crate) mod path_index;
pub(crate) mod persistence;
mod refresh_policy;
mod removal;
mod updates;
pub(crate) use updates::TagUpdates;
pub(crate) mod purge_worker;
mod sidebar_order;
mod view;
mod view_hides;
mod view_reload;

pub(crate) use definitions::PendingTagCascade;

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

fn tag_sort_key(tag: &FileTag) -> (i64, String) {
    (tag.position, tag.name.to_lowercase())
}

/// "Tag: %{name}" label for a tag view tab, shared by the app state (active-tab
/// sync, session restore) and the mono tab bar renderer so every surface shows
/// the same text (raw id when the tag is gone).
pub(crate) fn tag_view_title(tag_id: i64, tag_definitions: &FxHashMap<i64, FileTag>) -> String {
    let name = tag_definitions
        .get(&tag_id)
        .map(|tag| tag.name.clone())
        .unwrap_or_else(|| tag_id.to_string());
    rust_i18n::t!("tags.filter_active", name = name).to_string()
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
