//! File system watcher management
//!
//! This module handles the setup and management of the filesystem watcher
//! to detect external changes in the current directory.

use crate::app::state::{ImageViewerApp, WatcherFsProbeCacheEntry};
#[cfg(feature = "notify-watcher")]
use notify::{RecursiveMode, Watcher};
#[cfg(feature = "notify-watcher")]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WATCHER_FS_PROBE_CACHE_TTL: Duration = Duration::from_secs(600);

#[cfg(feature = "notify-watcher")]
fn normalize_watch_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

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
            .map(|dl| {
                self.rdcw_unreliable_drives
                    .get(&dl)
                    .copied()
                    .unwrap_or(false)
            })
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
                drive_letter,
                fs_name
            );
        }

        (fs_probe_ms, fs_probe_cache_hit, drive_letter)
    }

    /// Sets up monitoring for the current folder using per-folder notify-watcher.
    ///
    /// The consistency probe (background worker) provides additional drift detection
    /// for non-USN filesystems and cross-process changes missed by RDCW.
    pub fn watch_current_folder(&mut self) {
        let watch_start = Instant::now();
        let current_path = self.navigation_state.current_path.clone();

        // Skip virtual views that aren't real filesystem paths (e.g. "Lixeira", "Computador").
        if self.navigation_state.is_recycle_bin_view || self.navigation_state.is_computer_view {
            log::debug!(
                "[WATCHER] Skipping watch for virtual view: {}",
                current_path
            );
            return;
        }

        log::debug!("[WATCHER] Setting up for: {}", current_path);

        let path_buf = PathBuf::from(&current_path);
        let (fs_probe_ms, fs_probe_cache_hit, fs_probe_drive) =
            self.configure_watcher_fallback_mode(path_buf.as_path());

        // Use per-folder notify-watcher
        #[cfg(feature = "notify-watcher")]
        self.setup_notify_watcher();

        let total_ms = watch_start.elapsed().as_millis();
        if total_ms > 20 {
            log::warn!(
                "[PERF-WATCHER] watch_current_folder total={}ms fs_probe={}ms fs_cache_hit={} fs_cache_drive={:?} path={} fallback_polling={}",
                total_ms,
                fs_probe_ms,
                fs_probe_cache_hit,
                fs_probe_drive,
                current_path,
                self.watcher_fallback_polling,
            );
        }
    }

    /// Setup legacy notify-based watcher (fallback)
    #[cfg(feature = "notify-watcher")]
    fn setup_notify_watcher(&mut self) {
        let current_path = self.navigation_state.current_path.clone();
        let mut paths_to_watch = Vec::new();
        let mut seen_paths = HashSet::new();

        let mut push_watch_path = |path: String, label: &str| {
            let path_to_watch = if let Ok(p) = Path::new(&path).canonicalize() {
                log::debug!("[NOTIFY-WATCHER] Canonicalized {label} path: {:?}", p);
                p
            } else {
                log::warn!("[NOTIFY-WATCHER] Using original {label} path (canonicalize failed)");
                PathBuf::from(&path)
            };

            let normalized = normalize_watch_path(&path_to_watch);
            if seen_paths.insert(normalized) {
                paths_to_watch.push(path_to_watch);
            }
        };

        push_watch_path(current_path, "active");

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                if !snapshot.is_computer_view && !snapshot.is_recycle_bin_view {
                    push_watch_path(snapshot.path.clone(), "inactive dual-panel");
                }
            }
        }

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
            Ok(mut watcher) => {
                let mut watched_any = false;
                for path_to_watch in &paths_to_watch {
                    match watcher.watch(path_to_watch, RecursiveMode::NonRecursive) {
                        Ok(_) => {
                            watched_any = true;
                            log::debug!(
                                "[NOTIFY-WATCHER] Successfully watching: {:?}",
                                path_to_watch
                            );
                        }
                        Err(e) => {
                            log::error!(
                                "[NOTIFY-WATCHER] Failed to watch path: {:?} - Error: {}",
                                path_to_watch,
                                e
                            );
                        }
                    }
                }

                if watched_any {
                    self.watcher = Some(watcher);
                }
            }
            Err(e) => {
                log::error!("[NOTIFY-WATCHER] Failed to create watcher: {}", e);
            }
        }
    }
}
