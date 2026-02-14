use crate::app::state::ImageViewerApp;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

pub(super) struct WatcherPerfMarks {
    pub(super) watcher_start: Instant,
    pub(super) drive_events_done: Instant,
    pub(super) auto_reload_done: Instant,
}

impl ImageViewerApp {
    pub(super) fn process_watcher_events_and_auto_reload(
        &mut self,
        current_path_norm: &str,
    ) -> WatcherPerfMarks {
        let internal_cache_root_norm =
            dirs::data_local_dir().map(|d| Self::normalize_for_match(&d.join("MTT-File-Manager")));
        let internal_cache_root_prefix = internal_cache_root_norm
            .as_ref()
            .map(|root| format!("{root}\\"));

        let watcher_start = Instant::now();
        self.drive_watcher.check_pending_activation();

        let drive_events = self.drive_watcher.poll_events();
        let t_poll_done = Instant::now();
        let drive_event_count = drive_events.len();

        #[cfg(feature = "notify-watcher")]
        let drive_watcher_active = !drive_events.is_empty();

        for event in &drive_events {
            if let crate::infrastructure::drive_watcher::DriveWatcherEvent::DriveLost(drive_root) =
                event
            {
                log::warn!("[FS-WATCH] DriveLost signal received for: {:?}", drive_root);
                self.drive_state.last_drive_refresh = Instant::now();
                self.reload_drive_list_async();

                let drive_prefix = drive_root.to_string_lossy().to_string();
                if !self.navigation_state.is_computer_view
                    && !self.navigation_state.is_recycle_bin_view
                    && self
                        .navigation_state
                        .current_path
                        .starts_with(&drive_prefix)
                {
                    log::warn!(
                        "[FS-WATCH] Current path '{}' is on lost drive, redirecting to Este Computador",
                        self.navigation_state.current_path
                    );
                    self.directory_cache.clear();
                    self.drive_watcher.cleanup_unused_watchers(None);
                    self.navigate_to_computer();
                    return WatcherPerfMarks {
                        watcher_start,
                        drive_events_done: Instant::now(),
                        auto_reload_done: Instant::now(),
                    };
                }
            }
        }

        const MAX_EVENTS_INDIVIDUAL: usize = 50;
        const FLOOD_RELOAD_COOLDOWN_MS: u64 = 5000;

        let mut pending_disk_cache_invalidations: Vec<PathBuf> = Vec::new();
        let mut folders_with_changed_contents: HashSet<PathBuf> = HashSet::new();

        self.process_drive_events_batch(
            &drive_events,
            current_path_norm,
            internal_cache_root_norm.as_deref(),
            internal_cache_root_prefix.as_deref(),
            MAX_EVENTS_INDIVIDUAL,
            FLOOD_RELOAD_COOLDOWN_MS,
            &mut pending_disk_cache_invalidations,
            &mut folders_with_changed_contents,
        );

        self.apply_folder_content_change_invalidations(folders_with_changed_contents);

        let drive_events_done = Instant::now();
        if drive_events_done.duration_since(watcher_start).as_millis() > 50 {
            log::debug!(
                "[PERF-MSG] DriveWatcher: poll={}ms process={}ms events={}",
                t_poll_done.duration_since(watcher_start).as_millis(),
                drive_events_done.duration_since(t_poll_done).as_millis(),
                drive_event_count
            );
        }

        #[cfg(feature = "notify-watcher")]
        self.process_legacy_notify_events(
            drive_watcher_active,
            current_path_norm,
            internal_cache_root_norm.as_deref(),
            internal_cache_root_prefix.as_deref(),
            MAX_EVENTS_INDIVIDUAL,
            &mut pending_disk_cache_invalidations,
        );

        self.enqueue_disk_cache_invalidations(pending_disk_cache_invalidations);
        self.apply_watcher_reload_policy();

        let auto_reload_done = Instant::now();
        WatcherPerfMarks {
            watcher_start,
            drive_events_done,
            auto_reload_done,
        }
    }
}
