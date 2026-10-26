//! Main Application Module
//!
//! This module organizes the application logic into submodules:
//! - `state`: Defines the application state structure.
//! - `init`: Handles initialization and startup.
//! - `operations`: Implements business logic and operations.
//! - State sub-modules for better organization

pub mod batch_rename;
pub mod cache_state;
pub(crate) mod column_state;
pub mod default_file_manager_state;
pub mod disk_analysis_duplicates;
pub mod disk_analysis_model;
pub mod disk_analysis_query;
pub mod disk_analysis_sorting;
pub mod disk_analysis_state;
pub mod drag_drop_state;
pub mod drive_state;
pub mod dual_panel;
pub mod explorer_preferences;
pub mod file_hash;
pub mod file_operation_state;
pub mod folder_size_state;
pub(crate) mod folder_timestamp_state;
pub mod global_search_state;
pub mod init;
mod init_bootstrap;
mod init_post_startup;
mod init_preferences;
mod init_state_builders;
pub(crate) mod init_workers;
pub(crate) mod initial_indexing_notice;
pub mod layout_state;
pub(crate) mod live_file_size;
pub mod miller_columns_state;
pub mod navigation_state;
pub mod operations;
pub(crate) mod organizer_conflict_state;
pub(crate) mod organizer_history_state;
pub mod organizer_state;
pub mod shortcuts;
pub mod startup_folder;
pub mod startup_location;
#[cfg(test)]
mod startup_preferences_tests;
pub mod state;
pub mod tab_preferences;
pub mod tab_transfer;
pub mod ui_state;
pub(crate) mod update_state;

// Re-export navigation module for easy access
pub use operations::navigation;

pub use state::ImageViewerApp;
