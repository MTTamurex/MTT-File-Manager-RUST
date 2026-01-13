//! Main Application Module
//!
//! This module organizes the application logic into submodules:
//! - `state`: Defines the application state structure.
//! - `init`: Handles initialization and startup.
//! - `operations`: Implements business logic and operations (Replaced by `operations_new`).
//! - `message_handler`: Processes asynchronous messages.

pub mod init;
// pub mod message_handler; // MOVED to operations/message_handler.rs
// pub mod operations; // OLD
#[path = "operations_new/mod.rs"]
pub mod operations; // NEW mapped to old name for compatibility
pub mod state;

pub use state::ImageViewerApp;
