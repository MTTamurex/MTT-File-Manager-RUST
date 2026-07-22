use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderLockScope {
    CurrentFolder,
    Descendants,
}

impl FolderLockScope {
    pub fn preference_value(self) -> &'static str {
        match self {
            Self::CurrentFolder => "current_folder",
            Self::Descendants => "descendants",
        }
    }

    pub fn from_preference(value: &str) -> Self {
        match value {
            "descendants" => Self::Descendants,
            _ => Self::CurrentFolder,
        }
    }
}

/// Per-folder view preferences that are "locked" (pinned) by the user.
/// When a folder is locked, navigating to it applies these settings and
/// disables the UI controls so the view cannot be accidentally changed.
#[derive(Debug, Clone)]
pub struct FolderLock {
    pub view_mode: ViewMode,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub folders_position: FoldersPosition,
    pub scope: FolderLockScope,
}
