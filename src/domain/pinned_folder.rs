/// A folder pinned to the Quick Access section of the sidebar.
#[derive(Debug, Clone)]
pub struct PinnedFolder {
    /// Absolute path to the folder.
    pub path: String,
    /// Display name shown in the sidebar (typically the folder's file_name()).
    pub display_name: String,
    /// Position index for ordering (0-based, ascending).
    pub position: i64,
}
