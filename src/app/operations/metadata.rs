//! Media metadata handling
//!
//! This module handles requesting and formatting metadata for selected files.

use std::time::UNIX_EPOCH;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn refresh_selected_metadata(&mut self) {
        let current_file = self
            .selected_file
            .as_ref()
            .filter(|f| !f.is_dir)
            .map(|f| f.path.clone());

        match current_file {
            Some(path) => {
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

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
