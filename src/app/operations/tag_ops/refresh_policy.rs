use crate::domain::file_entry::{GroupMode, SortMode};
use crate::domain::list_columns::ColumnId;
use crate::domain::special_paths::tag_id_from_view_path;
use rustc_hash::FxHashSet;

pub(super) fn needs_refresh(
    path: &str,
    filter: Option<i64>,
    sort: SortMode,
    group: GroupMode,
    changed: &FxHashSet<i64>,
) -> bool {
    sort == SortMode::Column(ColumnId::Tags)
        || group == GroupMode::Column(ColumnId::Tags)
        || tag_id_from_view_path(path)
            .or(filter)
            .is_some_and(|id| changed.contains(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_folders_do_not_rebuild_but_tag_views_and_tag_columns_do() {
        let changed = [2].into_iter().collect();
        assert!(!needs_refresh(
            r"D:\Other",
            None,
            SortMode::Name,
            GroupMode::None,
            &changed
        ));
        assert!(!needs_refresh(
            &crate::domain::special_paths::tag_view_path(1),
            Some(1),
            SortMode::Name,
            GroupMode::None,
            &changed,
        ));
        assert!(needs_refresh(
            &crate::domain::special_paths::tag_view_path(2),
            Some(2),
            SortMode::Name,
            GroupMode::None,
            &changed,
        ));
        assert!(needs_refresh(
            r"D:\Other",
            None,
            SortMode::Column(ColumnId::Tags),
            GroupMode::None,
            &changed
        ));
        assert!(needs_refresh(
            r"D:\Other",
            None,
            SortMode::Name,
            GroupMode::Column(ColumnId::Tags),
            &changed
        ));
    }
}
