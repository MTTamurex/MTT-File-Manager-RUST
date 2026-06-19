/// Internal identifiers for virtual (non-filesystem) views.
///
/// These constants are used for internal routing, navigation history,
/// and state comparison. They are NOT displayed to the user — display
/// text comes from the i18n system via `t!()` calls.
pub const COMPUTER_VIEW_ID: &str = "::computer::";
pub const RECYCLE_BIN_VIEW_ID: &str = "::recycle_bin::";
pub const TAG_VIEW_PREFIX: &str = "::tag::";

pub fn tag_view_path(tag_id: i64) -> String {
    format!("{TAG_VIEW_PREFIX}{tag_id}")
}

pub fn tag_id_from_view_path(path: &str) -> Option<i64> {
    path.strip_prefix(TAG_VIEW_PREFIX)?.parse().ok()
}

pub fn is_tag_view_path(path: &str) -> bool {
    tag_id_from_view_path(path).is_some()
}

/// Returns `true` if the given path is a virtual view (not a real filesystem path).
pub fn is_virtual_path(path: &str) -> bool {
    path == COMPUTER_VIEW_ID || path == RECYCLE_BIN_VIEW_ID || is_tag_view_path(path)
}
