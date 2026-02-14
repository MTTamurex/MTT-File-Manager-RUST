//! Application layer - state management, worker coordination, and high-level business logic.
//! Follows .cursorrules: separation of concerns, clean architecture.

pub mod clipboard;
pub mod context_menu;
pub mod file_operations;
pub mod navigation;
pub mod notification;
pub mod renaming;
pub mod sorting;
pub mod watcher;

// Re-export for convenience
pub use clipboard::*;
pub use context_menu::*;
pub use navigation::*;
pub use notification::*;
pub use renaming::*;
pub use watcher::*;

pub use sorting::sort_items;

/// Backward-compatible wrapper for filtering items.
pub fn filter_items(
    items: &[crate::domain::file_entry::FileEntry],
    query: &str,
) -> Vec<crate::domain::file_entry::FileEntry> {
    sorting::filter_items(items, query)
}

/// Backward-compatible wrapper:
/// returns full list when query is empty.
pub fn filter_items_opt(
    items: &[crate::domain::file_entry::FileEntry],
    query: &str,
) -> Vec<crate::domain::file_entry::FileEntry> {
    sorting::filter_items_opt(items, query).unwrap_or_else(|| items.to_vec())
}

/// Compatibility alias kept for old call-sites.
pub fn filter_items_cow(
    items: &[crate::domain::file_entry::FileEntry],
    query: &str,
) -> Vec<crate::domain::file_entry::FileEntry> {
    sorting::filter_items(items, query)
}
