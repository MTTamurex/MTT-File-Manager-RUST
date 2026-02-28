//! Main Application Module
//!
//! This module organizes the application logic into submodules:
//! - `state`: Defines the application state structure.
//! - `init`: Handles initialization and startup.
//! - `operations`: Implements business logic and operations.
//! - State sub-modules for better organization

pub mod cache_state;
pub mod drive_state;
pub mod file_operation_state;
pub mod folder_size_state;
pub mod global_search_state;
pub mod init;
mod init_bootstrap;
mod init_post_startup;
mod init_preferences;
mod init_state_builders;
pub(crate) mod init_workers;
pub mod layout_state;
pub mod navigation_state;
pub mod operations;
pub mod state;
pub mod ui_state;

// Re-export navigation module for easy access
pub use operations::navigation;

pub use state::ImageViewerApp;
