//! Media metadata handling
//!
//! This module handles requesting and formatting metadata for selected files.
//!
//! PERFORMANCE CRITICAL: Uses timeout-protected I/O for OneDrive files to prevent
//! UI freezing on cloud-only files.

use std::time::UNIX_EPOCH;
use crate::app::state::ImageViewerApp;
use crate::infrastructure::onedrive::{self, IoTimeoutResult};

impl ImageViewerApp {
    pub fn refresh_selected_metadata(&mut self) {
        let current_file = self
            .selected_file
            .as_ref()
            .filter(|f| !f.is_dir)
            .map(|f| f.path.clone());

        match current_file {
            Some(path) => {
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

                // CRITICAL FIX: Use timeout-protected metadata for OneDrive
                // std::fs::metadata() can block indefinitely on cloud-only files
                let mtime = match onedrive::onedrive_metadata(&path) {
                    IoTimeoutResult::Ok(metadata) => {
                        metadata.modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    }
                    IoTimeoutResult::Timeout => {
                        eprintln!("[METADATA] Timeout reading metadata for {:?}, using cached", path);
                        // On timeout, use 0 to force cache miss and skip worker request
                        // This prevents blocking the UI thread
                        0
                    }
                    IoTimeoutResult::Err(_) => {
                        // Error reading metadata - use 0
                        0
                    }
                };

                if let Some((cached_mtime, meta)) = self.metadata_cache.get(&path) {
                    if *cached_mtime == mtime {
                        self.selected_metadata = Some((path, meta.clone()));
                        return;
                    }
                }

                if !self.metadata_loading.contains(&path) {
                    let _ = self.metadata_req_sender.send((path.clone(), mtime));
                    self.metadata_loading.insert(path.clone());
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
