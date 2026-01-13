//! Main Application Module
//!
//! This module organizes the application logic into submodules:
//! - `state`: Defines the application state structure.
//! - `init`: Handles initialization and startup.
//! - `operations`: Implements business logic and operations.
//! - `message_handler`: Processes asynchronous messages.

pub mod init;
pub mod message_handler;
pub mod operations;
pub mod state;

pub use state::ImageViewerApp;
