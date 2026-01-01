//! Application layer - state management, worker coordination, and high-level business logic
//! Follows .cursorrules: separation of concerns, clean architecture

pub mod state;
pub mod navigation;
pub mod clipboard;
pub mod context_menu;
pub mod watcher;
pub mod renaming;
pub mod notification;

// Re-export for convenience
pub use state::*;
pub use navigation::*;
pub use clipboard::*;
pub use context_menu::*;
pub use watcher::*;
pub use renaming::*;
pub use notification::*;
