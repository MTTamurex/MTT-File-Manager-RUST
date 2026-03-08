/// Internal identifiers for virtual (non-filesystem) views.
///
/// These constants are used for internal routing, navigation history,
/// and state comparison. They are NOT displayed to the user — display
/// text comes from the i18n system via `t!()` calls.
pub const COMPUTER_VIEW_ID: &str = "::computer::";
pub const RECYCLE_BIN_VIEW_ID: &str = "::recycle_bin::";

/// Returns `true` if the given path is a virtual view (not a real filesystem path).
pub fn is_virtual_path(path: &str) -> bool {
    path == COMPUTER_VIEW_ID || path == RECYCLE_BIN_VIEW_ID
}
