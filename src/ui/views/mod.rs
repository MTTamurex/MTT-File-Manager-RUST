//! View rendering modules
//! Follows .cursorrules: separation of concerns, < 300 lines per file

pub mod common;
pub mod computer_view;
pub mod grid_view;
pub mod list_view;
pub mod rectangle_selection;

// Re-export for convenience
pub use common::*;
pub use computer_view::*;
pub use grid_view::*;
pub use list_view::*;
