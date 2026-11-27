use super::*;
use crate::app::drive_state::merge_drive_info_query;
use std::cell::Cell;

struct FakeQueries {
    fail: bool,
    hardware_calls: Cell<u32>,
    // Confirm the first capacity was delivered before querying the next drive.
    results: Option<mpsc::Receiver<DriveInfoRefreshResult>>,
}

impl Queries for FakeQueries {
    fn drive_type(&self, _: &str) -> DriveType {
        DriveType::Fixed
    }
    fn capacity(&self, path: &str) -> windows::VolumeInfo {
        assert!(!self.fail, "simulated query failure");
        if path == "R:" {
            let result = self.results.as_ref().unwrap().try_recv().unwrap();
            assert!(!result.complete);
            assert_eq!(result.entries[0].path, "Q:");
        }
        windows::VolumeInfo {
            file_system: "NTFS".into(),
            total_space: 1000,
            free_space: 400,
            capacity_query_succeeded: true,
        }
    }
    fn hardware(&self, _: &str, _: DriveType) -> windows::physical_drive_info::HardwareFields {
        self.hardware_calls.set(self.hardware_calls.get() + 1);
        windows::physical_drive_info::HardwareFields {
            model: Some("updated model".into()),
            ..Default::default()
        }
    }
}

fn task() -> (Refresh, mpsc::Receiver<DriveInfoRefreshResult>, FakeQueries) {
    let (tx, rx) = mpsc::channel();
    let refresh = Refresh {
        scope: DriveInfoRefreshScope::Local,
        generation: 0,
        disks: vec![("Q:".into(), None)],
        tx,
        ctx: egui::Context::default(),
        hardware: Arc::default(),
        quiet: false,
    };
    let queries = FakeQueries {
        fail: false,
        hardware_calls: Cell::new(0),
        results: None,
    };
    (refresh, rx, queries)
}

fn cached() -> DriveInfo {
    DriveInfo {
        file_system: "NTFS".into(),
        total_space: 1000,
        free_space: 400,
        model: Some("updated model".into()),
        ..empty_info(DriveType::Fixed)
    }
}

#[test]
fn unchanged_values_emit_only_completion_and_do_not_stall_the_tracker() {
    let (mut refresh, rx, queries) = task();
    refresh.disks[0].1 = Some(cached());
    refresh.run(&queries);
    let results: Vec<_> = rx.try_iter().collect();
    assert_eq!(results.len(), 1);
    assert!(results[0].complete);
    assert!(results[0].entries.is_empty());
    let mut tracker = crate::app::drive_state::DriveInfoRefreshTracker::new(Instant::now());
    let generation = tracker.begin(DriveInfoRefreshScope::Local).unwrap();
    assert_eq!(results[0].generation, generation);
    tracker.finish(results[0].scope, generation, Instant::now());
    assert!(!tracker.is_pending(DriveInfoRefreshScope::Local));
    assert!(tracker.has_completed(DriveInfoRefreshScope::Local));
}

#[test]
fn capacity_is_published_before_a_later_volume_or_hardware_query() {
    let (mut refresh, rx, mut queries) = task();
    refresh.disks.push(("R:".into(), None));
    queries.results = Some(rx);
    refresh.run(&queries);
    assert_eq!(queries.hardware_calls.get(), 2);
}

#[test]
fn hardware_results_do_not_copy_old_capacity_or_health_fields() {
    let (mut refresh, rx, queries) = task();
    let mut old = cached();
    old.model = Some("old model".into());
    // Health records take precedence over static metadata. Test raw metadata
    // publication with no old snapshot, then merge with a newer health record.
    refresh.disks[0].1 = Some(old);
    refresh.run(&queries);
    let entry = rx.try_iter().flat_map(|r| r.entries).next().unwrap();
    assert!(!entry.capacity_query_succeeded);
    assert!(entry.info.file_system.is_empty());
    assert_eq!(entry.info.total_space, 0);
    assert!(entry.info.health.is_none());
    let mut latest = cached();
    latest.free_space = 250;
    latest.health = Some(crate::app::drive_state::tests_support::drive_health_snapshot('Q'));
    let merged = merge_drive_info_query(Some(&latest), entry.info, false);
    assert_eq!(merged.free_space, 250);
    assert_eq!(merged.health, latest.health);
}

#[test]
fn multiple_updates_in_one_ui_tick_keep_the_latest_metadata_per_root() {
    let mut results = Vec::new();
    let initial = cached();
    let mut latest = initial.clone();
    latest.model = Some("new model".into());
    super::super::record_latest_result(&mut results, r"Q:\".into(), initial);
    super::super::record_latest_result(&mut results, r"Q:\".into(), latest.clone());
    assert_eq!(results, vec![(r"Q:\".into(), latest)]);
}

#[test]
fn changed_capacity_is_delivered_and_failed_capacity_preserves_known_values() {
    let mut old = cached();
    old.free_space = 500;
    let (mut refresh, rx, queries) = task();
    refresh.disks[0].1 = Some(old.clone());
    refresh.run(&queries);
    let entry = rx.try_iter().flat_map(|r| r.entries).next().unwrap();
    assert!(entry.capacity_query_succeeded);
    assert_eq!(entry.info.free_space, 400);
    assert!(super::super::merge_changed_drive_info(
        Some(&old),
        empty_info(DriveType::Fixed),
        false,
    )
    .is_none());
}

#[test]
fn invalidated_worker_cannot_claim_hardware_queries() {
    let (mut refresh, _, queries) = task();
    refresh.generation = 99;
    refresh.run(&queries);
    assert_eq!(queries.hardware_calls.get(), 0);
}

#[test]
fn hardware_queries_are_throttled_even_when_response_is_partial() {
    let (mut refresh, rx, queries) = task();
    let schedule = refresh.hardware.clone();
    refresh.disks[0].1 = Some(cached());
    refresh.run(&queries);
    assert_eq!(queries.hardware_calls.get(), 1);
    let (mut second, _, second_queries) = task();
    second.hardware = schedule;
    second.disks[0].1 = Some(cached());
    second.run(&second_queries);
    assert_eq!(second_queries.hardware_calls.get(), 0);
    assert!(rx.try_iter().any(|r| r.complete));
}

#[test]
fn panic_still_emits_completion() {
    let (refresh, rx, mut queries) = task();
    queries.fail = true;
    refresh.run(&queries);
    assert!(rx.recv().unwrap().complete);
}

#[test]
fn quiet_completion_wakes_the_ui_only_after_a_published_change() {
    assert!(completion_wakes_ui(false, false));
    assert!(completion_wakes_ui(true, true));
    assert!(!completion_wakes_ui(true, false));
}

#[test]
fn quiet_refresh_reports_whether_anything_was_published() {
    let (mut refresh, rx, queries) = task();
    refresh.quiet = true;
    refresh.disks[0].1 = Some(cached());
    assert!(!refresh.query(&queries));
    assert!(rx.try_iter().next().is_none());

    let (refresh, rx, queries) = task();
    assert!(refresh.query(&queries));
    assert!(rx.try_iter().any(|result| !result.entries.is_empty()));
}
