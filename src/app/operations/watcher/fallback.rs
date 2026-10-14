use crate::app::state::{ImageViewerApp, WatcherFsProbeCacheEntry};
use std::path::Path;
use std::time::{Duration, Instant};

const WATCHER_FS_PROBE_CACHE_TTL: Duration = Duration::from_secs(600);

impl ImageViewerApp {
    pub(super) fn configure_watcher_fallback_mode(
        &mut self,
        path: &Path,
    ) -> (u128, bool, Option<char>) {
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

        if fs_name
            .as_deref()
            .is_some_and(crate::infrastructure::windows::lacks_cross_process_notifications)
        {
            self.watcher_fallback_polling = true;
            log::info!(
                "[WATCHER] FAT-family folder uses consistency polling without a persistent directory watcher"
            );
            return (fs_probe_ms, fs_probe_cache_hit, drive_letter);
        }
        if is_usn {
            self.watcher_fallback_polling = false;
            return (fs_probe_ms, fs_probe_cache_hit, drive_letter);
        }

        let already_known_bad = drive_letter
            .map(|dl| {
                self.rdcw_unreliable_drives
                    .get(&dl)
                    .copied()
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        self.watcher_fallback_polling = true;
        if already_known_bad {
            log::info!(
                "[WATCHER] Drive {:?} (fs={:?}): RDCW previously verified as unreliable; active polling",
                drive_letter,
                fs_name
            );
        } else {
            log::info!(
                "[WATCHER] Drive {:?} (fs={:?}): RDCW unverified; verification probing active",
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
}
