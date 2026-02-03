//! Main Application Module
//!
//! This module organizes the application logic into submodules:
//! - `state`: Defines the application state structure.
//! - `init`: Handles initialization and startup.
//! - `operations`: Implements business logic and operations.
//! - Sub-módulos de estado para melhor organização

pub mod cache_state;
pub mod init;
pub mod navigation_state;
pub mod operations;
pub mod state;
pub mod ui_state;
pub mod worker_state;

// Re-export navigation module for easy access
pub use operations::navigation;

pub use state::ImageViewerApp;
