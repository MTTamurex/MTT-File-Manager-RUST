use crate::app::state::ImageViewerApp;
use rustc_hash::FxHashSet;

pub(crate) fn parse_sidebar_tag_order(value: &str) -> Vec<i64> {
    value
        .split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect()
}

pub(crate) fn serialize_sidebar_tag_order(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

pub(crate) fn reconcile_sidebar_tag_order(saved: &[i64], canonical: &[i64]) -> Vec<i64> {
    let valid: FxHashSet<i64> = canonical.iter().copied().collect();
    let mut seen = FxHashSet::default();
    let mut reconciled = Vec::with_capacity(canonical.len());

    for id in saved.iter().chain(canonical) {
        if valid.contains(id) && seen.insert(*id) {
            reconciled.push(*id);
        }
    }

    reconciled
}

/// Moves `tag_id` within its sibling chain in the flat sidebar order.
///
/// The sidebar renders tags grouped under their parent; only the relative
/// order of same-parent tags matters. `before_tag_id` must therefore be a
/// sibling (same parent) — cross-level drops are rejected. `None` moves the
/// tag to the end of its sibling chain.
fn move_within_siblings(
    ids: &mut Vec<i64>,
    tag_id: i64,
    before_tag_id: Option<i64>,
    parent_of: impl Fn(i64) -> Option<i64>,
) -> bool {
    let tag_parent = parent_of(tag_id);
    if let Some(target) = before_tag_id {
        if target == tag_id || parent_of(target) != tag_parent {
            return false;
        }
    }
    if !ids
        .iter()
        .any(|id| *id != tag_id && parent_of(*id) == tag_parent)
    {
        return false;
    }

    let Some(source_index) = ids.iter().position(|id| *id == tag_id) else {
        return false;
    };

    let mut insert_index = match before_tag_id {
        Some(target) => ids.iter().position(|id| *id == target).unwrap_or(ids.len()),
        None => ids
            .iter()
            .rposition(|id| *id != tag_id && parent_of(*id) == tag_parent)
            .map(|position| position + 1)
            .unwrap_or(ids.len()),
    };
    if insert_index > source_index {
        insert_index -= 1;
    }

    let previous = ids.clone();
    let id = ids.remove(source_index);
    ids.insert(insert_index, id);
    *ids != previous
}

impl ImageViewerApp {
    pub fn reorder_sidebar_tag(&mut self, tag_id: i64, before_tag_id: Option<i64>) {
        let parent_of = |id: i64| self.tag_definitions.get(&id).and_then(|tag| tag.parent_id);
        if move_within_siblings(&mut self.sidebar_tag_ids, tag_id, before_tag_id, parent_of) {
            self.save_preferences();
            self.ui_ctx.request_repaint();
        }
    }

    pub(crate) fn sync_sidebar_tag_order(&mut self) {
        let reconciled = reconcile_sidebar_tag_order(&self.sidebar_tag_ids, &self.sorted_tag_ids);
        if reconciled != self.sidebar_tag_ids {
            self.sidebar_tag_ids = reconciled;
            self.save_preferences();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        move_within_siblings, parse_sidebar_tag_order, reconcile_sidebar_tag_order,
        serialize_sidebar_tag_order,
    };

    /// Parent map for the tests: 1 parents 2 and 3; 4 is a root; 5 parents 6.
    fn parent_of(id: i64) -> Option<i64> {
        match id {
            2 | 3 => Some(1),
            6 => Some(5),
            _ => None,
        }
    }

    #[test]
    fn parses_and_serializes_order() {
        let parsed = parse_sidebar_tag_order("5, 2,invalid,-1,3");
        assert_eq!(parsed, vec![5, 2, 3]);
        assert_eq!(serialize_sidebar_tag_order(&parsed), "5,2,3");
    }

    #[test]
    fn reconciles_removed_duplicate_and_new_tags() {
        assert_eq!(
            reconcile_sidebar_tag_order(&[8, 3, 3, 99, 2], &[1, 2, 3, 4, 8]),
            vec![8, 3, 2, 1, 4]
        );
    }

    #[test]
    fn moves_tags_within_their_sibling_chain() {
        // Flat: [1, 2, 3, 4] where 2 and 3 are children of 1.
        let mut ids = vec![1, 2, 3, 4];
        assert!(move_within_siblings(&mut ids, 3, Some(2), parent_of));
        assert_eq!(ids, vec![1, 3, 2, 4]);

        // Move to the end of the sibling chain.
        let mut ids = vec![1, 2, 3, 4];
        assert!(move_within_siblings(&mut ids, 2, None, parent_of));
        assert_eq!(ids, vec![1, 3, 2, 4]);

        // Roots reorder among themselves without touching child entries.
        let mut ids = vec![1, 2, 3, 4];
        assert!(move_within_siblings(&mut ids, 4, Some(1), parent_of));
        assert_eq!(ids, vec![4, 1, 2, 3]);
    }

    #[test]
    fn rejects_cross_level_and_invalid_moves() {
        let mut ids = vec![1, 2, 3, 4, 5, 6];
        // Dropping a root before a child of another tag is not a sibling move.
        assert!(!move_within_siblings(&mut ids, 4, Some(2), parent_of));
        // A child cannot be dropped before a root.
        assert!(!move_within_siblings(&mut ids, 2, Some(4), parent_of));
        assert!(!move_within_siblings(&mut ids, 2, Some(2), parent_of));
        assert!(!move_within_siblings(&mut ids, 9, Some(2), parent_of));
        assert!(!move_within_siblings(&mut ids, 2, Some(9), parent_of));
        // A lone child has no sibling to swap with.
        assert!(!move_within_siblings(&mut ids, 6, None, parent_of));
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6]);
    }
}
