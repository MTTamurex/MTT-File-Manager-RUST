//! Application layer - state management, worker coordination, and high-level business logic.
//! Follows .cursorrules: separation of concerns, clean architecture.

pub mod clipboard;
pub mod context_menu;
pub(crate) mod disk_analysis_policy;
pub mod file_operations;
pub(crate) mod folder_export;
pub mod grouping;
pub mod navigation;
pub mod notification;
pub mod renaming;
pub mod sidebar_tree;
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
