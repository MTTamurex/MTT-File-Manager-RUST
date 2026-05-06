//! Application operations split into focused modules.
//!
//! Each module handles a specific area of functionality:
//! - `file_ops`: File deletion, creation, renaming
//! - `clipboard_ops`: Copy, cut, paste operations
//! - `navigation`: Path navigation and history
//! - `folder_loading`: Async folder scanning and filtering
//! - `view_setup`: Computer view, recycle bin view setup
//! - `recycle_bin_ops`: Recycle bin specific operations
//! - `tabs`: Tab synchronization
//! - `watcher`: File system watcher management
//! - `preferences`: Save/load user preferences
//! - `thumbnails`: Thumbnail loading requests
//! - `icons`: Icon loading and caching
//! - `metadata`: Media metadata handling
//! - `selection`: Selection state management
//! - `context_menu`: Context menu population
//! - `window`: Window handle management
//! - `message_handler`: Async message processing
//! - `ui_rendering`: Rendering functions for file lists
//! - `trait_impls`: Implementation of UI traits for App

pub mod clipboard_ops;
pub mod context_menu;
pub mod drag_drop;
pub mod dual_panel_ops;
pub mod file_ops;
pub mod folder_loading;
pub mod folder_lock_ops;
pub mod global_search;
pub mod icons;
pub mod message_handler;
pub mod metadata;
pub mod navigation;
pub mod pinned_folder_ops;
pub mod preferences;
pub mod rectangle_selection;
pub mod recycle_bin_ops;
pub mod selection;
pub mod shutdown;
pub mod tabs;
pub mod thumbnails;
pub mod trait_impls;
pub mod ui_rendering;
pub mod view_setup;
pub mod watcher;
pub mod window;
