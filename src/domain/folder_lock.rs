use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};

/// Per-folder view preferences that are "locked" (pinned) by the user.
/// When a folder is locked, navigating to it applies these settings and
/// disables the UI controls so the view cannot be accidentally changed.
#[derive(Debug, Clone)]
pub struct FolderLock {
    pub view_mode: ViewMode,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub folders_position: FoldersPosition,
}
