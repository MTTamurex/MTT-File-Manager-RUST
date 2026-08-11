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
                    self.cached_file_system_for_drive(path, dl)
                }
            } else {
                self.cached_file_system_for_drive(path, dl)
            }
        } else {
            (None, false, false)
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

    fn cached_file_system_for_drive(
        &mut self,
        path: &Path,
        drive_letter: char,
    ) -> (Option<String>, bool, bool) {
        let drive_root = format!("{}:\\", drive_letter);
        let fs_name = self
            .drive_state
            .drive_info_cache
            .get(&drive_root)
            .and_then(|info| (!info.file_system.is_empty()).then(|| info.file_system.clone()));
        let is_usn = fs_name
            .as_deref()
            .map(crate::infrastructure::windows::is_usn_filesystem)
            .unwrap_or(false);

        if fs_name.is_some() {
            self.watcher_fs_probe_cache.insert(
                drive_letter,
                WatcherFsProbeCacheEntry {
                    file_system: fs_name.clone(),
                    is_usn,
                    probed_at: Instant::now(),
                },
            );
        } else {
            self.watcher_fs_probe_cache.remove(&drive_letter);
            log::debug!(
                "[WATCHER] Filesystem metadata unavailable in cache for {:?}; using non-blocking fallback probing",
                path
            );
        }

        (fs_name, is_usn, false)
    }

    /// Sets up monitoring for the current folder using per-folder notify-watcher.
    ///
    /// The consistency probe (background worker) provides additional drift detection
    /// for non-USN filesystems and cross-process changes missed by RDCW.
    pub fn watch_current_folder(&mut self) {
        let watch_start = Instant::now();
        let current_path = self.navigation_state.current_path.clone();

        // Skip virtual views that aren't real filesystem paths (e.g. "Lixeira", "Computador").
        if crate::domain::special_paths::is_virtual_path(&current_path) {
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

        let mut push_watch_path = |path: String, label: &str| {
            let path_to_watch = PathBuf::from(&path);

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

        if paths_to_watch.is_empty() {
            self.watcher = None;
            self.notify_watcher_setup_pending = false;
            self.notify_watcher_setup_retry_after = None;
            return;
        }

        // Every request invalidates older setup results, even if the thread cap
        // forces this request to use consistency polling temporarily.
        self.notify_watcher_setup_request_id = self.notify_watcher_setup_request_id.wrapping_add(1);
        let request_id = self.notify_watcher_setup_request_id;

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
        let active_path = paths_to_watch[0].clone();

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
                        let event = crate::app::state::TimestampedNotifyEvent {
                            received_at: std::time::Instant::now(),
                            result: res,
                            overflow: false,
                        };
                        match tx.try_send(event) {
                            Ok(()) => {}
                            Err(crossbeam_channel::TrySendError::Full(dropped)) => {
                                // EST-01: bounded channel saturated under a sustained
                                // event storm. Drop the oldest queued event and
                                // coalesce the burst into a single overflow marker;
                                // the UI applies one debounced reload instead of
                                // processing every dropped event.
                                let _ = rx_for_overflow.try_recv();
                                let marker_paths = match &dropped.result {
                                    Ok(evt) => evt.paths.clone(),
                                    Err(err) => err.paths.clone(),
                                };
                                let marker = notify::Event {
                                    kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
                                    paths: marker_paths,
                                    attrs: Default::default(),
                                };
                                let mut overflow_event =
                                    crate::app::state::TimestampedNotifyEvent {
                                        received_at: std::time::Instant::now(),
                                        result: Ok(marker),
                                        overflow: true,
                                    };
                                loop {
                                    match tx.try_send(overflow_event) {
                                        Ok(()) => break,
                                        Err(crossbeam_channel::TrySendError::Full(returned)) => {
                                            let _ = rx_for_overflow.try_recv();
                                            overflow_event = returned;
                                        }
                                        Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
                        }
                        ctx_for_events.request_repaint();
                    });

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
                            watched_count == paths_to_watch.len(),
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
