//! File system watcher management
//!
//! This module handles the setup and management of the filesystem watcher
//! to detect external changes in the current directory.

use crate::app::state::{ImageViewerApp, WatcherFsProbeCacheEntry};
#[cfg(feature = "notify-watcher")]
use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WATCHER_FS_PROBE_CACHE_TTL: Duration = Duration::from_secs(600);

impl ImageViewerApp {
    fn configure_watcher_fallback_mode(&mut self, path: &Path) -> (u128, bool, Option<char>) {
        self.watcher_fallback_last_probe = Instant::now();
        self.watcher_fallback_signature = None;

        let fs_probe_start = Instant::now();
        let drive_letter = crate::infrastructure::windows::extract_drive_letter(path);

        let (fs_name, is_usn, fs_probe_cache_hit) = if let Some(dl) = drive_letter {
            let cached_entry = self.watcher_fs_probe_cache.get(&dl).cloned();
            if let Some(entry) = cached_entry {
                if entry.probed_at.elapsed() <= WATCHER_FS_PROBE_CACHE_TTL {
                    (entry.file_system, entry.is_usn, true)
                } else {
                    let fs_name = crate::infrastructure::windows::get_file_system_for_path(path);
                    let is_usn = fs_name
                        .as_deref()
                        .map(crate::infrastructure::windows::is_usn_filesystem)
                        .unwrap_or(true); // unknown FS → assume reliable
                    self.watcher_fs_probe_cache.insert(
                        dl,
                        WatcherFsProbeCacheEntry {
                            file_system: fs_name.clone(),
                            is_usn,
                            probed_at: Instant::now(),
                        },
                    );
                    (fs_name, is_usn, false)
                }
            } else {
                let fs_name = crate::infrastructure::windows::get_file_system_for_path(path);
                let is_usn = fs_name
                    .as_deref()
                    .map(crate::infrastructure::windows::is_usn_filesystem)
                    .unwrap_or(true); // unknown FS → assume reliable
                self.watcher_fs_probe_cache.insert(
                    dl,
                    WatcherFsProbeCacheEntry {
                        file_system: fs_name.clone(),
                        is_usn,
                        probed_at: Instant::now(),
                    },
                );
                (fs_name, is_usn, false)
            }
        } else {
            let fs_name = crate::infrastructure::windows::get_file_system_for_path(path);
            let is_usn = fs_name
                .as_deref()
                .map(crate::infrastructure::windows::is_usn_filesystem)
                .unwrap_or(true); // unknown FS → assume reliable
            (fs_name, is_usn, false)
        };

        let fs_probe_ms = fs_probe_start.elapsed().as_millis();

        self.watcher_fallback_fs = fs_name.clone();

        if is_usn {
            // NTFS / ReFS — USN journal + reliable RDCW. Zero polling overhead.
            self.watcher_fallback_polling = false;
            return (fs_probe_ms, fs_probe_cache_hit, drive_letter);
        }

        // Non-USN filesystem: check if we already learned this drive is unreliable.
        let already_known_bad = drive_letter
            .map(|dl| self.rdcw_unreliable_drives.get(&dl).copied().unwrap_or(false))
            .unwrap_or(false);

        if already_known_bad {
            // We previously detected RDCW drift on this drive → active polling.
            self.watcher_fallback_polling = true;
            log::info!(
                "[WATCHER] Drive {:?} (fs={:?}): RDCW previously verified as unreliable → active polling",
                drive_letter, fs_name
            );
        } else {
            // RDCW not yet proven bad. Enable verification mode: slow probing
            // that checks for drift without invalidating caches.
            // If drift is found, maybe_poll_non_usn_consistency will escalate.
            self.watcher_fallback_polling = true;
            log::info!(
                "[WATCHER] Drive {:?} (fs={:?}): RDCW unverified → verification probing active",
                drive_letter, fs_name
            );
        }

        (fs_probe_ms, fs_probe_cache_hit, drive_letter)
    }

    /// Sets up monitoring for the current folder
    ///
    /// DUAL USE:
    /// 1. New: Drive-wide watcher (monitors entire drive, filters by prefix)
    /// 2. Legacy: notify-watcher (monitors specific folder)
    ///
    /// The drive watcher is more efficient for fast navigation since it doesn't need
    /// to recreate the watcher on every folder change within the same drive.
    pub fn watch_current_folder(&mut self) {
        let watch_start = Instant::now();
        let current_path = self.navigation_state.current_path.clone();

        // Skip virtual views that aren't real filesystem paths (e.g. "Lixeira", "Computador").
        if self.navigation_state.is_recycle_bin_view || self.navigation_state.is_computer_view {
            log::debug!("[WATCHER] Skipping watch for virtual view: {}", current_path);
            return;
        }

        log::debug!("[WATCHER] Setting up for: {}", current_path);

        // Try using drive-wide watcher first
        let path_buf = PathBuf::from(&current_path);
        let (fs_probe_ms, fs_probe_cache_hit, fs_probe_drive) =
            self.configure_watcher_fallback_mode(path_buf.as_path());

        // Drive watcher only works for local drives (C:\, D:\, etc.)
        // Does NOT work for UNC paths (\\server\share) or network drives
        let is_local_drive = path_buf.to_string_lossy().chars().nth(1) == Some(':');

        if is_local_drive {
            log::debug!(
                "[WATCHER] Using DRIVE-WATCHER for local drive: {:?}",
                path_buf
            );
            self.drive_watcher.watch_path(path_buf);

            // If drive watcher is active on USN filesystems (NTFS/ReFS), avoid duplicates.
            // On non-USN filesystems (exFAT/FAT), keep notify as backup for resilience.
            if self.drive_watcher.is_active() {
                if !self.watcher_fallback_polling {
                    log::debug!("[WATCHER] Drive watcher is active - skipping notify-watcher");
                    // Drop notify watcher if it exists to save resources
                    #[cfg(feature = "notify-watcher")]
                    if self.watcher.is_some() {
                        log::debug!("[WATCHER] Dropping notify-watcher to save resources");
                        self.watcher = None;
                    }

                    let total_ms = watch_start.elapsed().as_millis();
                    if total_ms > 20 {
                        log::warn!(
                            "[PERF-WATCHER] watch_current_folder total={}ms fs_probe={}ms fs_cache_hit={} fs_cache_drive={:?} path={} local_drive={} drive_active={} fallback_polling={}",
                            total_ms,
                            fs_probe_ms,
                            fs_probe_cache_hit,
                            fs_probe_drive,
                            current_path,
                            is_local_drive,
                            self.drive_watcher.is_active(),
                            self.watcher_fallback_polling,
                        );
                    }
                    return;
                }
                log::debug!(
                    "[WATCHER] Drive watcher active + non-USN fallback enabled - keeping notify backup"
                );
            }
        } else {
            log::debug!("[WATCHER] UNC/Network path detected - using notify-watcher only");
        }

        // FALLBACK: Use notify-watcher for UNC paths or if drive watcher failed
        #[cfg(feature = "notify-watcher")]
        self.setup_notify_watcher();

        let total_ms = watch_start.elapsed().as_millis();
        if total_ms > 20 {
            log::warn!(
                "[PERF-WATCHER] watch_current_folder total={}ms fs_probe={}ms fs_cache_hit={} fs_cache_drive={:?} path={} local_drive={} drive_active={} fallback_polling={}",
                total_ms,
                fs_probe_ms,
                fs_probe_cache_hit,
                fs_probe_drive,
                current_path,
                is_local_drive,
                self.drive_watcher.is_active(),
                self.watcher_fallback_polling,
            );
        }
    }

    /// Setup legacy notify-based watcher (fallback)
    #[cfg(feature = "notify-watcher")]
    fn setup_notify_watcher(&mut self) {
        let current_path = self.navigation_state.current_path.clone();

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
                            event.kind,
                            event.paths
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
                        path_to_watch,
                        e
                    );
                }
            },
            Err(e) => {
                log::error!("[NOTIFY-WATCHER] Failed to create watcher: {}", e);
            }
        }
    }
}
