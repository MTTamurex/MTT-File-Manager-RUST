//! Application layer - state management, worker coordination, and high-level business logic
//! Follows .cursorrules: separation of concerns, clean architecture

pub mod clipboard;
pub mod context_menu;
pub mod file_operations;
pub mod navigation;
pub mod notification;
pub mod renaming;
pub mod sorting;
pub mod sorting_optimized;
pub mod state;
pub mod watcher;

// Re-export for convenience
pub use clipboard::*;
pub use context_menu::*;
pub use navigation::*;
pub use notification::*;
pub use renaming::*;
pub use state::*;
pub use watcher::*;

// Re-export funções otimizadas
pub use sorting_optimized::{filter_items, filter_items_cow, filter_items_opt, sort_items};
