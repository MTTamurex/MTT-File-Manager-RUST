//! File system watcher management
//!
//! This module handles the setup and management of the filesystem watcher
//! to detect external changes in the current directory.

use crate::app::state::ImageViewerApp;
#[cfg(feature = "notify-watcher")]
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};

impl ImageViewerApp {
    /// Sets up monitoring for the current folder
    ///
    /// DUAL USE:
    /// 1. New: Drive-wide watcher (monitors entire drive, filters by prefix)
    /// 2. Legacy: notify-watcher (monitors specific folder)
    ///
    /// The drive watcher is more efficient for fast navigation since it doesn't need
    /// to recreate the watcher on every folder change within the same drive.
    pub fn watch_current_folder(&mut self) {
        let current_path = self.current_path.clone();
        log::debug!("[WATCHER] Setting up for: {}", current_path);

        // Try using drive-wide watcher first (File Pilot optimization)
        let path_buf = PathBuf::from(&current_path);

        // Drive watcher only works for local drives (C:\, D:\, etc.)
        // Does NOT work for UNC paths (\\server\share) or network drives
        let is_local_drive = path_buf.to_string_lossy().chars().nth(1) == Some(':');

        if is_local_drive {
            log::debug!(
                "[WATCHER] Using DRIVE-WATCHER for local drive: {:?}",
                path_buf
            );
            self.drive_watcher.watch_path(path_buf);

            // If drive watcher is active, do NOT use notify (avoid duplicates)
            if self.drive_watcher.is_active() {
                log::debug!("[WATCHER] Drive watcher is active - skipping notify-watcher");
                // Drop notify watcher if it exists to save resources
                #[cfg(feature = "notify-watcher")]
                if self.watcher.is_some() {
                    log::debug!("[WATCHER] Dropping notify-watcher to save resources");
                    self.watcher = None;
                }
                return;
            }
        } else {
            log::debug!("[WATCHER] UNC/Network path detected - using notify-watcher only");
        }

        // FALLBACK: Use notify-watcher for UNC paths or if drive watcher failed
        #[cfg(feature = "notify-watcher")]
        self.setup_notify_watcher();
    }

    /// Setup legacy notify-based watcher (fallback)
    #[cfg(feature = "notify-watcher")]
    fn setup_notify_watcher(&mut self) {
        let current_path = self.current_path.clone();

        // Canonicalize the path for Windows compatibility
        let path_to_watch = if let Ok(p) = Path::new(&current_path).canonicalize() {
            log::debug!("[NOTIFY-WATCHER] Canonicalized path: {:?}", p);
            p
        } else {
            log::warn!("[NOTIFY-WATCHER] Using original path (canonicalize failed)");
            PathBuf::from(&current_path)
        };

        // Drop the previous watcher if it exists
        if self.watcher.is_some() {
            log::debug!("[NOTIFY-WATCHER] Dropping previous watcher");
            self.watcher = None;
        }

        // Create or recreate the watcher
        let tx = self.fs_event_sender.clone();
        let ctx_clone = self.ui_ctx.clone();

        let watcher_result =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                match &res {
                    Ok(event) => {
                        log::trace!(
                            "[NOTIFY-WATCHER] Event received: kind={:?}, paths={:?}",
                            event.kind, event.paths
                        );
                    }
                    Err(e) => {
                        log::error!("[NOTIFY-WATCHER] Event error: {}", e);
                    }
                }
                let _ = tx.send(res);
                ctx_clone.request_repaint();
            });

        match watcher_result {
            Ok(mut watcher) => match watcher.watch(&path_to_watch, RecursiveMode::NonRecursive) {
                Ok(_) => {
                    log::debug!(
                        "[NOTIFY-WATCHER] Successfully watching: {:?}",
                        path_to_watch
                    );
                    self.watcher = Some(watcher);
                }
                Err(e) => {
                    log::error!(
                        "[NOTIFY-WATCHER] Failed to watch path: {:?} - Error: {}",
                        path_to_watch, e
                    );
                }
            },
            Err(e) => {
                log::error!("[NOTIFY-WATCHER] Failed to create watcher: {}", e);
            }
        }
    }
}
