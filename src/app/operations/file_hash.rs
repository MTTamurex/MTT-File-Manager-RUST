//! File hash (SHA-256) on-demand handling.
//!
//! Mirrors the existing metadata / live-size worker pattern: the UI only
//! triggers a request and reads from a cache; the heavy computation runs
//! in a dedicated background worker, off the render thread.

use crate::app::state::ImageViewerApp;
use std::path::PathBuf;

impl ImageViewerApp {
    pub fn enqueue_file_hash_for_selected(&mut self, path: PathBuf) {
        let Some(file) = self
            .selected_file
            .as_ref()
            .filter(|f| f.path == path && crate::app::file_hash::can_hash_file(f))
        else {
            return;
        };

        let status = crate::app::file_hash::FileHashStatus {
            modified: file.modified,
            size: file.size,
        };

        // Refuse to hash files currently being downloaded / written — opening
        // them with shared-read could fight the downloader (same rationale
        // used for the metadata worker).
        if crate::infrastructure::windows::file_flags::is_file_unsafe_to_read_fast(&path) {
            self.selected_file_hash = Some((
                path,
                status.modified,
                status.size,
                Err("unsafe to read".to_string()),
            ));
            return;
        }

        self.selected_file_hash = None;

        let _ = crate::app::file_hash::try_enqueue_file_hash(
            &path,
            status,
            &mut self.file_hash_loading,
            &self.file_hash_req_sender,
        );
    }
}
