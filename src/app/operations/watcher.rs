//! File system watcher management
//!
//! This module handles the setup and management of the filesystem watcher
//! to detect external changes in the current directory.

use crate::app::state::ImageViewerApp;
#[cfg(feature = "notify-watcher")]
use notify::RecursiveMode;
#[cfg(feature = "notify-watcher")]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod fallback;

#[cfg(feature = "notify-watcher")]
mod event_handler;
#[cfg(feature = "notify-watcher")]
mod polling_policy;

/// EST-02: cap on concurrently alive "notify-watcher-setup" threads. Setup
/// performs blocking kernel calls (`CreateFileW`/RDCW handle acquisition) that
/// can hang indefinitely on dead SMB/FUSE volumes; repeated navigation across
/// such paths previously accumulated one blocked thread per navigation. When
/// the cap is reached the setup is skipped — the next navigation (or watcher
/// reconfiguration) retries.
#[cfg(feature = "notify-watcher")]
const MAX_NOTIFY_SETUP_THREADS: usize = 2;
#[cfg(feature = "notify-watcher")]
static LIVE_NOTIFY_SETUP_THREADS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "notify-watcher")]
fn normalize_watch_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

#[cfg(feature = "notify-watcher")]
fn miller_ancestor_watch_paths(path: &Path) -> Vec<PathBuf> {
    path.ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .collect()
}

impl ImageViewerApp {
    /// Sets up monitoring for the current folder using per-folder notify-watcher.
    ///
    /// The consistency probe (background worker) provides additional drift detection
    /// for non-USN filesystems and cross-process changes missed by RDCW.
    pub fn watch_current_folder(&mut self) {
        let watch_start = Instant::now();
        let current_path = self.navigation_state.current_path.clone();

        // Skip virtual views that aren't real filesystem paths (e.g. "Lixeira", "Computador").
        if crate::domain::special_paths::is_virtual_path(&current_path)
            || crate::domain::network_path::unc_server_name(&current_path).is_some()
        {
            #[cfg(feature = "notify-watcher")]
            {
                self.notify_watcher_setup_request_id =
                    self.notify_watcher_setup_request_id.wrapping_add(1);
                self.notify_watcher_setup_pending = false;
                self.notify_watcher_setup_retry_after = None;
                self.watcher = None;
            }
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
        self.queue_notify_watcher_setup();

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
    fn queue_notify_watcher_setup(&mut self) {
        let current_path = self.navigation_state.current_path.clone();
        let mut paths_to_watch = Vec::new();
        let mut seen_paths = HashSet::new();
        let mut suspended_watch_paths = false;

        let mut push_watch_path = |path: String, label: &str| {
            if crate::domain::network_path::unc_server_name(&path).is_some() {
                return;
            }
            let path_to_watch = PathBuf::from(&path);
            if crate::infrastructure::windows::extract_drive_letter(&path_to_watch)
                .is_some_and(crate::infrastructure::removal_aware_watcher::is_suspended)
            {
                suspended_watch_paths = true;
                return;
            }

            let normalized = normalize_watch_path(&path_to_watch);
            if seen_paths.insert(normalized) {
                log::debug!("[NOTIFY-WATCHER] Queued {label} path: {:?}", path_to_watch);
                paths_to_watch.push(path_to_watch);
            }
        };

        push_watch_path(current_path.clone(), "active");
        if matches!(self.view_mode, crate::domain::file_entry::ViewMode::Miller) {
            for ancestor in miller_ancestor_watch_paths(Path::new(&current_path)) {
                push_watch_path(
                    ancestor.to_string_lossy().into_owned(),
                    "active Miller ancestor",
                );
            }
        }

        if self.dual_panel_enabled {
            if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
                if !crate::domain::special_paths::is_virtual_path(&snapshot.path) {
                    push_watch_path(snapshot.path.clone(), "inactive dual-panel");
                    if matches!(
                        snapshot.view_mode,
                        crate::domain::file_entry::ViewMode::Miller
                    ) {
                        for ancestor in miller_ancestor_watch_paths(Path::new(&snapshot.path)) {
                            push_watch_path(
                                ancestor.to_string_lossy().into_owned(),
                                "inactive Miller ancestor",
                            );
                        }
                    }
                }
            }
        }

        // Invalidate stale setup results even when no native watches remain
        // or the thread cap forces this request to use polling temporarily.
        self.notify_watcher_setup_request_id = self.notify_watcher_setup_request_id.wrapping_add(1);
        let request_id = self.notify_watcher_setup_request_id;

        if paths_to_watch.is_empty() {
            self.watcher = None;
            self.notify_watcher_setup_pending = suspended_watch_paths;
            self.notify_watcher_setup_retry_after =
                suspended_watch_paths.then(|| Instant::now() + Duration::from_secs(5));
            return;
        }

        // Reserve a setup slot atomically before spawning. Incrementing inside
        // the new thread allowed concurrent callers to all pass the cap check.
        let reserved = LIVE_NOTIFY_SETUP_THREADS
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |live| (live < MAX_NOTIFY_SETUP_THREADS).then_some(live + 1),
            )
            .is_ok();
        if !reserved {
            // Keep consistency polling enabled while notify setup is unavailable,
            // including on NTFS/ReFS where it is normally disabled.
            self.watcher_fallback_polling = true;
            self.notify_watcher_setup_pending = true;
            self.notify_watcher_setup_retry_after = None;
            log::warn!(
                "[NOTIFY-WATCHER] Setup thread cap ({}) reached; skipping setup for {:?} (retry on next navigation)",
                MAX_NOTIFY_SETUP_THREADS,
                current_path
            );
            return;
        }

        self.notify_watcher_setup_pending = false;
        self.notify_watcher_setup_retry_after = None;
        let setup_tx = self.notify_watcher_setup_sender.clone();
        let tx = self.fs_event_sender.clone();
        // EST-01: the producer needs a receiver clone to drop the oldest
        // queued event when the bounded channel saturates (crossbeam senders
        // cannot drain).
        let rx_for_overflow = self.fs_event_receiver.clone();
        let ctx_for_events = self.ui_ctx.clone();
        let ctx_for_setup = self.ui_ctx.clone();
        let active_path = PathBuf::from(current_path);

        let spawn_result = std::thread::Builder::new()
            .name("notify-watcher-setup".to_string())
            .spawn(move || {
                // EST-02 guard: count this thread while it is alive, including
                // panic unwinds.
                struct SetupThreadGuard;
                impl Drop for SetupThreadGuard {
                    fn drop(&mut self) {
                        LIVE_NOTIFY_SETUP_THREADS
                            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                let _setup_guard = SetupThreadGuard;

                let suspended_watch_paths = suspended_watch_paths
                    || paths_to_watch.iter().any(|path| {
                        crate::infrastructure::removal_aware_watcher::path_is_suspended(path)
                    });
                let paths_to_watch = polling_policy::native_watch_paths(paths_to_watch);
                if paths_to_watch.is_empty() {
                    let _ = setup_tx.send(crate::app::state::NotifyWatcherSetupResult {
                        request_id,
                        watcher: None,
                        active_watched: false,
                        complete: !suspended_watch_paths,
                    });
                    ctx_for_setup.request_repaint();
                    return;
                }

                let watcher_result =
                    crate::infrastructure::removal_aware_watcher::RemovalAwareWatcher::new(
                        event_handler::handler(tx, rx_for_overflow, ctx_for_events),
                    );

                let watcher_to_install = match watcher_result {
                    Ok(mut watcher) => {
                        let mut watched_any = false;
                        let mut active_watched = false;
                        let mut watched_count = 0usize;
                        for path_to_watch in &paths_to_watch {
                            match watcher.watch(path_to_watch, RecursiveMode::NonRecursive) {
                                Ok(_) => {
                                    watched_any = true;
                                    active_watched |= path_to_watch == &active_path;
                                    watched_count += 1;
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

                        (
                            watched_any.then_some(watcher),
                            active_watched,
                            watched_count == paths_to_watch.len() && !suspended_watch_paths,
                        )
                    }
                    Err(e) => {
                        log::error!("[NOTIFY-WATCHER] Failed to create watcher: {}", e);
                        (None, false, false)
                    }
                };

                let _ = setup_tx.send(crate::app::state::NotifyWatcherSetupResult {
                    request_id,
                    watcher: watcher_to_install.0,
                    active_watched: watcher_to_install.1,
                    complete: watcher_to_install.2,
                });
                ctx_for_setup.request_repaint();
            });

        if let Err(error) = spawn_result {
            LIVE_NOTIFY_SETUP_THREADS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            self.notify_watcher_setup_pending = true;
            self.notify_watcher_setup_retry_after = Some(Instant::now() + Duration::from_secs(5));
            self.watcher_fallback_polling = true;
            log::error!("[NOTIFY-WATCHER] Failed to spawn setup thread: {}", error);
        }
    }

    #[cfg(feature = "notify-watcher")]
    pub(crate) fn poll_notify_watcher_setup(&mut self) {
        if self
            .watcher
            .as_ref()
            .is_some_and(|watcher| watcher.needs_refresh())
        {
            self.watcher = None;
            self.queue_notify_watcher_setup();
        }
        while let Ok(result) = self.notify_watcher_setup_receiver.try_recv() {
            if result.request_id != self.notify_watcher_setup_request_id {
                log::debug!(
                    "[NOTIFY-WATCHER] Dropping stale watcher setup result request_id={}",
                    result.request_id
                );
                continue;
            }

            if !result.active_watched {
                self.watcher_fallback_polling = true;
            }
            if !result.complete {
                self.notify_watcher_setup_pending = true;
                self.notify_watcher_setup_retry_after =
                    Some(Instant::now() + Duration::from_secs(5));
            }
            self.watcher = result.watcher;
        }

        if self.notify_watcher_setup_pending
            && self
                .notify_watcher_setup_retry_after
                .is_none_or(|retry_after| Instant::now() >= retry_after)
            && LIVE_NOTIFY_SETUP_THREADS.load(std::sync::atomic::Ordering::Acquire)
                < MAX_NOTIFY_SETUP_THREADS
        {
            self.queue_notify_watcher_setup();
        }
    }
}

#[cfg(all(test, feature = "notify-watcher"))]
mod tests {
    use super::*;

    #[test]
    fn miller_watches_each_ancestor_of_the_focused_directory() {
        assert_eq!(
            miller_ancestor_watch_paths(Path::new(r"C:\A\B")),
            vec![PathBuf::from(r"C:\A"), PathBuf::from(r"C:\")]
        );
    }

    #[test]
    fn miller_drive_root_has_no_ancestor_watch_paths() {
        assert!(miller_ancestor_watch_paths(Path::new(r"C:\")).is_empty());
    }
}
