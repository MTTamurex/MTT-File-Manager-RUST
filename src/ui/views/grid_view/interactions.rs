use super::GridViewAction;

pub(super) fn resolve_grid_action(
    clicked_item: Option<usize>,
    double_clicked_item: Option<usize>,
    secondary_clicked_item: Option<usize>,
    empty_area_clicked: bool,
    bg_secondary_clicked: bool,
) -> Option<GridViewAction> {
    if let Some(idx) = double_clicked_item {
        return Some(GridViewAction::DoubleClick(idx));
    }

    if let Some(idx) = secondary_clicked_item {
        return Some(GridViewAction::SecondaryClick(idx));
    }

    if secondary_clicked_item.is_none() && bg_secondary_clicked {
        return Some(GridViewAction::EmptyAreaSecondaryClick);
    }

    if let Some(idx) = clicked_item {
        return Some(GridViewAction::Click(idx));
    }

    if empty_area_clicked {
        return Some(GridViewAction::EmptyAreaClick);
    }

    None
}
