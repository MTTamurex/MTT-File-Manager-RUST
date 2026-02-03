//! UI Rendering bridges - simplified coordination module
//!
//! This module provides bridge implementations between App state and UI views,
//! delegating actual rendering to specialized view modules.

pub mod list_bridge;
pub mod grid_bridge;
pub mod item_slot_bridge;

// Re-export commonly used types
pub use list_bridge::*;
pub use grid_bridge::*;
