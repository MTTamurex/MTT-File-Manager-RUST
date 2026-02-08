//! UI Rendering bridges - simplified coordination module
//!
//! This module provides bridge implementations between App state and UI views,
//! delegating actual rendering to specialized view modules.

pub mod grid_bridge;
pub mod item_slot_bridge;
pub mod list_bridge;

// Re-export commonly used types
pub use grid_bridge::*;
pub use list_bridge::*;
