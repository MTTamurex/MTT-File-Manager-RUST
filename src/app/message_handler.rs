// Message handler module - processes messages from background workers
// This will be populated with message processing logic from the main event loop

//! Message handling for asynchronous events.
//!
//! This module implements the `process_messages` method, which acts as the main
//! event loop consumer, processing signals from background workers (icons, thumbnails, filesystem).

use std::time::Duration;

use super::state::ImageViewerApp;

impl ImageViewerApp {
    // Placeholder for now - message processing will be extracted from impl eframe::App
    // Methods to be moved here:
    // - process_device_events()
    // - process_fs_events()
    // - process_thumbnail_messages()
    // - process_metadata_messages()
    // - process_folder_preview_messages()
    // - etc.
}
