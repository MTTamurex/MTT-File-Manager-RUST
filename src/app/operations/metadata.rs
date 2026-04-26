//! Media metadata handling
//!
//! This module handles requesting and formatting metadata for selected files.
//!
//! PERFORMANCE CRITICAL: Uses mtime from FileEntry (already loaded during folder scan)
//! instead of calling std::fs::metadata() on the UI thread, which can block indefinitely
//! on OneDrive cloud-only files.

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn refresh_selected_metadata(&mut self) {
        let current_file_info = self
            .selected_file
            .as_ref()
            .filter(|f| !f.is_dir)
            .map(|f| (f.path.clone(), f.modified));

        match current_file_info {
            Some((path, file_mtime)) => {
                // EVENT-DRIVEN: If same file and already loaded, trust the cache.
                // DriveWatcher clears last_metadata_path when the file changes,
                // which triggers a re-fetch on the next frame. No polling needed.
                if self.last_metadata_path.as_ref() == Some(&path) {
                    if let Some((_, meta)) = self.metadata_cache.get(&path) {
                        self.selected_metadata = Some((path, meta.clone()));
                    }
                    return;
                }

                // New file selected or DriveWatcher signaled change — initial load
                self.last_metadata_path = Some(path.clone());
                self.last_metadata_refresh = std::time::Instant::now();

                // CRITICAL FIX: Use mtime from FileEntry.modified (already loaded during
                // folder scan in a background thread) instead of calling onedrive_metadata()
                // which spin-waits up to 100ms on the UI thread. After long inactivity,
                // OneDrive dehydrates files and metadata() ALWAYS times out, causing
                // 100ms blocking per frame.
                let mtime = file_mtime;

                if let Some((cached_mtime, meta)) = self.metadata_cache.get(&path) {
                    if *cached_mtime == mtime {
                        self.selected_metadata = Some((path, meta.clone()));
                        return;
                    }
                }

                if !self.metadata_loading.contains(&path) {
                    // Don't send metadata requests for files that are being
                    // downloaded/written — the extraction would open them with
                    // COM APIs that lack FILE_SHARE_WRITE, killing the download.
                    // Uses _fast variant (no CreateFileW) to avoid blocking UI thread.
                    if !crate::infrastructure::windows::file_flags::is_file_unsafe_to_read_fast(
                        &path,
                    ) {
                        let _ = self.metadata_req_sender.send((path.clone(), mtime));
                        self.metadata_loading.insert(path.clone());
                    }
                }

                if !matches!(self.selected_metadata.as_ref(), Some((p, _)) if p == &path) {
                    self.selected_metadata = None;
                }
            }
            None => {
                self.selected_metadata = None;
                self.last_metadata_path = None;
            }
        }
    }

    pub fn format_media_duration(ticks_100ns: u64) -> String {
        // 1 tick = 100ns; 10_000_000 ticks = 1s
        let total_seconds = ticks_100ns / 10_000_000;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        } else {
            format!("{:02}:{:02}", minutes, seconds)
        }
    }

    pub fn format_bitrate(bps: u32) -> String {
        let bps = bps as f64;
        if bps >= 1_000_000.0 {
            format!("{:.1} Mbps", bps / 1_000_000.0)
        } else if bps >= 1_000.0 {
            format!("{:.0} Kbps", bps / 1_000.0)
        } else {
            format!("{:.0} bps", bps)
        }
    }

    pub fn approximate_bitrate(size_bytes: u64, duration_100ns: u64) -> Option<u32> {
        if duration_100ns == 0 {
            return None;
        }
        let seconds = duration_100ns as f64 / 10_000_000.0;
        if seconds <= 0.0 {
            return None;
        }
        let bits_per_sec = (size_bytes as f64 * 8.0) / seconds;
        Some(bits_per_sec.max(0.0) as u32)
    }
}
