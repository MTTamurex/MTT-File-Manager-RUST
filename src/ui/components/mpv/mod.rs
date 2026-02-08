//! MPV video preview module
//!
//! This module provides MPV-based video playback functionality.
//! All public types and functions are re-exported here for backward compatibility.

// Re-export all public types
pub use state::{MpvState, TrackInfo};
pub use utils::format_time;

// Sub-modules
pub mod event_loop;
pub mod filters;
pub mod playback;
pub mod state;
pub mod utils;
