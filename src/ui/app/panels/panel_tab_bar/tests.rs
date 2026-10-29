use super::tab_tooltip_text;

#[test]
fn virtual_tab_tooltips_use_the_display_title() {
    let tag_path = crate::domain::special_paths::tag_view_path(2);
    assert_eq!(tab_tooltip_text(&tag_path, "Tag: Movies"), "Tag: Movies");
    assert_eq!(
        tab_tooltip_text(crate::domain::special_paths::COMPUTER_VIEW_ID, "Computer"),
        "Computer"
    );
}

#[test]
fn filesystem_tab_tooltips_keep_the_full_path() {
    assert_eq!(tab_tooltip_text("folder", "folder"), "folder");
}
