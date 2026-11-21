//! Shows the initial-indexing notice only after observing a real volume scan.

use mtt_search_protocol::VolumeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitialIndexingNoticeEvent {
    None,
    ProgressChanged,
    Completed,
    CacheReused,
}

pub(crate) struct InitialIndexingNotice {
    installation_generation: Option<String>,
    last_total_indexed: Option<u64>,
    observed_full_scan: bool,
    completed: bool,
    completion_persistence_finished: bool,
}

impl InitialIndexingNotice {
    pub(crate) fn new(installation_generation: Option<String>) -> Self {
        Self {
            installation_generation,
            last_total_indexed: None,
            observed_full_scan: false,
            completed: false,
            completion_persistence_finished: false,
        }
    }

    pub(crate) fn is_tracking(&self) -> bool {
        self.installation_generation.is_some() && !self.completed
    }

    pub(crate) fn completion_generation(&self) -> Option<&str> {
        (self.completed && !self.completion_persistence_finished)
            .then_some(())
            .and(self.installation_generation.as_deref())
    }

    pub(crate) fn mark_completion_persistence_finished(&mut self) {
        self.completion_persistence_finished = true;
    }

    pub(crate) fn observe_status(
        &mut self,
        available: bool,
        total_indexed: u64,
        volumes: &[VolumeStatus],
    ) -> InitialIndexingNoticeEvent {
        if !self.is_tracking() || !available {
            return InitialIndexingNoticeEvent::None;
        }

        self.observed_full_scan |= volumes.iter().any(|volume| {
            volume.state == "scanning"
                && matches!(volume.phase.as_str(), "scanning_mft" | "filesystem_scan")
        });

        if !self.observed_full_scan {
            if initial_indexing_complete(available, volumes) {
                self.completed = true;
                return InitialIndexingNoticeEvent::CacheReused;
            }
            return InitialIndexingNoticeEvent::None;
        }

        let progress_changed =
            self.last_total_indexed.replace(total_indexed) != Some(total_indexed);

        if initial_indexing_complete(available, volumes) {
            // Publish the terminal count before replacing the progress toast
            // with the completion message on the next status poll.
            if progress_changed {
                return InitialIndexingNoticeEvent::ProgressChanged;
            }
            self.completed = true;
            return InitialIndexingNoticeEvent::Completed;
        }

        if progress_changed {
            InitialIndexingNoticeEvent::ProgressChanged
        } else {
            InitialIndexingNoticeEvent::None
        }
    }
}

fn initial_indexing_complete(available: bool, volumes: &[VolumeStatus]) -> bool {
    available && !volumes.is_empty() && volumes.iter().all(|volume| volume.state == "ready")
}

#[cfg(test)]
mod tests {
    use super::{InitialIndexingNotice, InitialIndexingNoticeEvent};
    use mtt_search_protocol::VolumeStatus;

    fn volume(state: &str, sizes_loading: bool) -> VolumeStatus {
        VolumeStatus {
            drive_letter: 'C',
            state: state.to_string(),
            files_indexed: 10,
            phase: if state == "scanning" {
                "scanning_mft".to_string()
            } else {
                state.to_string()
            },
            phase_progress: None,
            phase_total: None,
            sizes_loading,
        }
    }

    #[test]
    fn completes_when_every_volume_index_is_ready() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));

        assert_eq!(
            notice.observe_status(true, 10, &[volume("scanning", false)]),
            InitialIndexingNoticeEvent::ProgressChanged
        );
        assert_eq!(
            notice.observe_status(true, 10, &[volume("ready", true)]),
            InitialIndexingNoticeEvent::Completed
        );
        assert!(!notice.is_tracking());
    }

    #[test]
    fn does_not_complete_without_a_service_volume() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));

        assert_eq!(
            notice.observe_status(true, 0, &[]),
            InitialIndexingNoticeEvent::None
        );
        assert!(notice.is_tracking());
    }

    #[test]
    fn publishes_final_count_before_completion_when_ready_count_changes() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));

        assert_eq!(
            notice.observe_status(true, 222_000, &[volume("scanning", false)]),
            InitialIndexingNoticeEvent::ProgressChanged
        );
        assert_eq!(
            notice.observe_status(true, 2_000_000, &[volume("ready", false)]),
            InitialIndexingNoticeEvent::ProgressChanged
        );
        assert!(notice.is_tracking());
        assert_eq!(
            notice.observe_status(true, 2_000_000, &[volume("ready", false)]),
            InitialIndexingNoticeEvent::Completed
        );
    }

    #[test]
    fn reinstall_with_existing_cache_never_reports_initial_indexing() {
        let mut notice = InitialIndexingNotice::new(Some("new-generation".to_string()));
        for (phase, count) in [
            ("starting", 0),
            ("loading_cache", 1),
            ("loading_cache", 10),
            ("catching_up", 11),
            ("loading_sizes", 11),
            ("filesystem_refresh", 12),
        ] {
            let mut status = volume("scanning", false);
            status.phase = phase.to_string();
            assert_eq!(
                notice.observe_status(true, count, &[status]),
                InitialIndexingNoticeEvent::None
            );
            assert!(notice.completion_generation().is_none());
        }
        assert_eq!(
            notice.observe_status(true, 11, &[volume("ready", false)]),
            InitialIndexingNoticeEvent::CacheReused
        );
        assert!(!notice.is_tracking());
        assert_eq!(notice.completion_generation(), Some("new-generation"));
        notice.mark_completion_persistence_finished();
        assert!(notice.completion_generation().is_none());
    }

    #[test]
    fn already_ready_service_completes_silently_on_first_status() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));
        assert_eq!(
            notice.observe_status(true, 10, &[volume("ready", false)]),
            InitialIndexingNoticeEvent::CacheReused
        );
        assert_eq!(
            notice.observe_status(true, 10, &[volume("ready", false)]),
            InitialIndexingNoticeEvent::None
        );
    }

    #[test]
    fn fallback_filesystem_scan_is_still_reported() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));
        let mut status = volume("scanning", false);
        status.phase = "filesystem_scan".to_string();
        assert_eq!(
            notice.observe_status(true, 10, &[status]),
            InitialIndexingNoticeEvent::ProgressChanged
        );
    }

    #[test]
    fn progress_is_not_persisted_as_completion() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));
        assert!(notice.completion_generation().is_none());
        notice.observe_status(true, 10, &[volume("scanning", false)]);
        assert!(notice.completion_generation().is_none());
        notice.observe_status(true, 10, &[volume("ready", false)]);
        assert_eq!(notice.completion_generation(), Some("generation"));
    }

    #[test]
    fn unavailable_service_cannot_trigger_a_notice_from_stale_status() {
        let mut notice = InitialIndexingNotice::new(Some("generation".to_string()));
        assert_eq!(
            notice.observe_status(false, 10, &[volume("scanning", false)]),
            InitialIndexingNoticeEvent::None
        );
        assert!(notice.is_tracking());
        assert!(notice.completion_generation().is_none());
    }
}
