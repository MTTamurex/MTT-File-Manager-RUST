use std::time::Duration;

use super::{NonUsnScanCadence, NonUsnWaitResult};
use crate::volume_indices::VolumeIndexHandle;

pub(super) struct ScanSchedule {
    scan_immediately: bool,
}

impl ScanSchedule {
    pub(super) fn new(cached_index: Option<&VolumeIndexHandle>) -> Self {
        Self {
            scan_immediately: cached_index.is_none(),
        }
    }

    /// Run immediately only when no index was published. Every subsequent
    /// scan, including startup with a cache, must pass through the same wait.
    pub(super) fn wait_until_due(
        &mut self,
        interval: &mut Duration,
        cadence: NonUsnScanCadence,
        wait: impl FnOnce(Duration) -> NonUsnWaitResult,
        wait_for_shutdown: impl FnOnce(Duration) -> bool,
    ) -> bool {
        if std::mem::take(&mut self.scan_immediately) {
            return true;
        }
        match wait(*interval) {
            NonUsnWaitResult::Shutdown => false,
            NonUsnWaitResult::Timeout => true,
            NonUsnWaitResult::Triggered => {
                *interval = cadence.periodic_interval;
                !wait_for_shutdown(cadence.min_trigger_gap)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::volume_indexers::test_support::{index, TestDb};
    use crate::volume_indices::handle_from;

    fn cadence() -> NonUsnScanCadence {
        super::super::non_usn_scan_cadence("exFAT")
    }

    fn cached_handle() -> VolumeIndexHandle {
        handle_from(index())
    }

    #[test]
    fn persisted_cache_is_ready_and_startup_waits_without_requesting_a_scan() {
        let fixture = TestDb::new();
        let mut source = index();
        assert!(source.insert_record(20, "file.txt", 5, false, false));
        fixture.db.save_volume(&source, |_, _| {}).unwrap();
        let volume = crate::volume_indexers::IndexVolume {
            drive_letter: 'R',
            volume_id: "volume-id".into(),
            volume_serial: 123,
            file_system: "exFAT".into(),
        };
        let estimate = super::super::cache::matching_record_estimate(&fixture.db, &volume);
        let loaded =
            super::super::cache::load_cached_index(&fixture.db, &volume, estimate).unwrap();
        let handle = handle_from(loaded);
        let mut schedule = ScanSchedule::new(Some(&handle));
        let mut interval = cadence().periodic_interval;
        assert!(!schedule.wait_until_due(
            &mut interval,
            cadence(),
            |delay| {
                assert_eq!(delay, Duration::from_secs(120));
                assert!(matches!(
                    handle.read().state,
                    crate::file_index::IndexState::Ready
                ));
                assert_eq!(handle.read().records.len(), 1);
                NonUsnWaitResult::Shutdown
            },
            |_| panic!("no change notification"),
        ));
        assert_eq!(handle.read().records.len(), 1);
    }

    #[test]
    fn missing_cache_scans_once_immediately_then_waits() {
        let mut schedule = ScanSchedule::new(None);
        let mut interval = cadence().periodic_interval;
        assert!(schedule.wait_until_due(
            &mut interval,
            cadence(),
            |_| panic!("a fresh install must not wait for its initial scan"),
            |_| panic!("no change notification"),
        ));
        assert!(!schedule.wait_until_due(
            &mut interval,
            cadence(),
            |_| NonUsnWaitResult::Shutdown,
            |_| panic!("no change notification"),
        ));
    }

    #[test]
    fn cached_index_is_refreshed_after_each_periodic_timeout() {
        let handle = cached_handle();
        let mut schedule = ScanSchedule::new(Some(&handle));
        let mut interval = cadence().periodic_interval;
        for _ in 0..2 {
            assert!(schedule.wait_until_due(
                &mut interval,
                cadence(),
                |delay| {
                    assert_eq!(delay, Duration::from_secs(120));
                    NonUsnWaitResult::Timeout
                },
                |_| panic!("timeouts must not use the change debounce"),
            ));
        }
    }

    #[test]
    fn change_notification_resets_backoff_and_honors_debounce() {
        let handle = cached_handle();
        let mut schedule = ScanSchedule::new(Some(&handle));
        let mut interval = Duration::from_secs(360);
        assert!(schedule.wait_until_due(
            &mut interval,
            cadence(),
            |delay| {
                assert_eq!(delay, Duration::from_secs(360));
                NonUsnWaitResult::Triggered
            },
            |delay| {
                assert_eq!(delay, Duration::from_secs(15));
                false
            },
        ));
        assert_eq!(interval, Duration::from_secs(120));
    }

    #[test]
    fn shutdown_during_change_debounce_prevents_a_scan() {
        let handle = cached_handle();
        let mut schedule = ScanSchedule::new(Some(&handle));
        let mut interval = cadence().periodic_interval;
        assert!(!schedule.wait_until_due(
            &mut interval,
            cadence(),
            |_| NonUsnWaitResult::Triggered,
            |_| true,
        ));
    }
}
