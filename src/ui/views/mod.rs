//! View rendering modules
//! Follows .cursorrules: separation of concerns, < 300 lines per file

pub mod common;
pub mod list_view;
pub mod grid_view;
pub mod computer_view;

// Re-export for convenience
pub use common::*;
pub use list_view::*;
pub use grid_view::*;
pub use computer_view::*;
