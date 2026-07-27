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

fn move_before(ids: &mut Vec<i64>, tag_id: i64, before_tag_id: Option<i64>) -> bool {
    if before_tag_id == Some(tag_id) {
        return false;
    }

    let Some(source_index) = ids.iter().position(|id| *id == tag_id) else {
        return false;
    };
    if before_tag_id.is_some_and(|target| !ids.contains(&target)) {
        return false;
    }

    let previous = ids.clone();
    ids.remove(source_index);
    let insert_index = before_tag_id
        .and_then(|target| ids.iter().position(|id| *id == target))
        .unwrap_or(ids.len());
    ids.insert(insert_index, tag_id);
    *ids != previous
}

impl ImageViewerApp {
    pub fn reorder_sidebar_tag(&mut self, tag_id: i64, before_tag_id: Option<i64>) {
        if move_before(&mut self.sidebar_tag_ids, tag_id, before_tag_id) {
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
        move_before, parse_sidebar_tag_order, reconcile_sidebar_tag_order,
        serialize_sidebar_tag_order,
    };

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
    fn moves_tags_before_targets_and_to_end() {
        let mut ids = vec![1, 2, 3, 4];
        assert!(move_before(&mut ids, 4, Some(2)));
        assert_eq!(ids, vec![1, 4, 2, 3]);

        assert!(move_before(&mut ids, 1, Some(3)));
        assert_eq!(ids, vec![4, 2, 1, 3]);

        assert!(move_before(&mut ids, 2, None));
        assert_eq!(ids, vec![4, 1, 3, 2]);
    }

    #[test]
    fn rejects_invalid_or_unchanged_moves() {
        let mut ids = vec![1, 2, 3];
        assert!(!move_before(&mut ids, 2, Some(2)));
        assert!(!move_before(&mut ids, 9, Some(1)));
        assert!(!move_before(&mut ids, 2, Some(9)));
        assert!(!move_before(&mut ids, 1, Some(2)));
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
