//! Tracks the one-time initial-indexing notice for an installer generation.

use mtt_search_protocol::VolumeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitialIndexingNoticeEvent {
    None,
    ProgressChanged,
    Completed,
}

pub(crate) struct InitialIndexingNotice {
    installation_generation: Option<String>,
    last_total_indexed: Option<u64>,
    completed: bool,
    completion_persistence_finished: bool,
}

impl InitialIndexingNotice {
    pub(crate) fn new(installation_generation: Option<String>) -> Self {
        Self {
            installation_generation,
            last_total_indexed: None,
            completed: false,
            completion_persistence_finished: false,
        }
    }

    pub(crate) fn is_tracking(&self) -> bool {
        self.installation_generation.is_some() && !self.completed
    }

    pub(crate) fn completion_generation(&self) -> Option<&str> {
        (!self.completion_persistence_finished)
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
        if !self.is_tracking() {
            return InitialIndexingNoticeEvent::None;
        }

        if initial_indexing_complete(available, volumes) {
            self.completed = true;
            return InitialIndexingNoticeEvent::Completed;
        }

        if self.last_total_indexed.replace(total_indexed) != Some(total_indexed) {
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
            phase: state.to_string(),
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
            InitialIndexingNoticeEvent::ProgressChanged
        );
        assert!(notice.is_tracking());
    }
}
