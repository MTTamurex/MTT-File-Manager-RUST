/// Background refreshes keep the complete listing visible, even when empty.
/// Replacing it with the initial-loading screen would also remove list headers.
pub(super) fn show_initial_loading(
    is_loading: bool,
    items_empty: bool,
    hold_visible_items: bool,
) -> bool {
    is_loading && items_empty && !hold_visible_items
}

#[cfg(test)]
mod tests {
    use super::show_initial_loading;

    #[test]
    fn empty_folder_refresh_keeps_the_view_and_headers() {
        assert!(!show_initial_loading(true, true, true));
        assert!(!show_initial_loading(false, true, false));
    }

    #[test]
    fn initial_navigation_and_hard_refresh_show_loading() {
        assert!(show_initial_loading(true, true, false));
    }

    #[test]
    fn populated_views_remain_visible_while_loading() {
        assert!(!show_initial_loading(true, false, true));
        assert!(!show_initial_loading(true, false, false));
    }
}
