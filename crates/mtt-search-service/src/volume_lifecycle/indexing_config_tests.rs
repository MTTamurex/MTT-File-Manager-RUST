use super::*;
use crate::file_index::{IndexState, VolumeIndex};
use crate::indexing_progress::IndexingProgress;
use crate::volume_indices;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

#[path = "indexing_review_tests.rs"]
mod review;

struct Fixture {
    lifecycle: Arc<VolumeLifecycle>,
    db: Arc<IndexDb>,
    events: mpsc::Receiver<()>,
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = Arc::new(IndexDb::open(&directory.path().join("index.db")).unwrap());
        let (wake, events) = mpsc::channel();
        let lifecycle = VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            volume_indices::new_shared(),
            Arc::new(IndexingProgress::new()),
            wake,
        );
        lifecycle.initialize_drive_config(db.clone()).unwrap();
        Self {
            lifecycle,
            db,
            events,
            directory,
        }
    }

    fn connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.directory.path().join("index.db")).unwrap()
    }
}

fn volume(letter: char) -> DiscoveredVolume {
    DiscoveredVolume {
        drive_letter: letter,
        volume_id: format!("test-volume-{letter}"),
        volume_serial: letter as u32,
        label: String::new(),
        file_system: "NTFS".into(),
        usn_supported: true,
    }
}

fn index(volume: &DiscoveredVolume) -> VolumeIndex {
    let mut index = VolumeIndex::empty(volume.drive_letter);
    index.volume_id = volume.volume_id.clone();
    index.volume_serial = volume.volume_serial;
    index.state = IndexState::Ready;
    assert!(index.insert_record_untracked(10, "file.txt", 5, false, false));
    index
}

#[test]
fn default_enabled_and_absent_preferences_are_preserved_and_sorted() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    assert!(lifecycle.is_indexing_enabled('R'));
    lifecycle.set_drive_indexing('t', false).unwrap();
    lifecycle.record_drive_discovery(
        &[volume('S'), volume('R')],
        lifecycle.drive_config_revision(),
    );
    let response = lifecycle.drive_indexing_config_response();
    response.validate().unwrap();
    let SearchResponse::DriveIndexingConfig {
        drives,
        service_discovery_complete,
        enabled_service_volumes,
    } = response
    else {
        panic!("unexpected response")
    };
    assert!(service_discovery_complete);
    assert_eq!(enabled_service_volumes, vec!['R', 'S']);
    assert_eq!(
        drives,
        vec![
            DriveIndexingEntry {
                drive_letter: 'R',
                enabled: true
            },
            DriveIndexingEntry {
                drive_letter: 'S',
                enabled: true
            },
            DriveIndexingEntry {
                drive_letter: 'T',
                enabled: false
            },
        ]
    );
}

#[test]
fn restart_applies_preferences_before_worker_admission() {
    let fixture = Fixture::new();
    fixture.lifecycle.set_drive_indexing('R', false).unwrap();
    let (wake, _) = mpsc::channel();
    let restarted = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake,
    );
    restarted
        .initialize_drive_config(fixture.db.clone())
        .unwrap();
    assert!(restarted.claim_worker(&volume('R')).is_none());
    assert!(restarted.claim_worker(&volume('S')).is_some());
}

#[test]
fn disable_removes_search_and_blocks_stale_discovery_and_publication() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let stop = ticket.stop_signal();
    assert!(stop
        .publish_index(&lifecycle.indices, index(&volume))
        .is_some());
    assert_eq!(lifecycle.acquire_search_access().handles().len(), 1);

    lifecycle.set_drive_indexing('R', false).unwrap();
    assert!(stop.is_stopped());
    assert!(lifecycle.acquire_volume('R').is_err());
    assert!(lifecycle.acquire_search_access().handles().is_empty());
    assert!(volume_indices::snapshot_handles(&lifecycle.indices).is_empty());
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&volume));
    assert!(lifecycle.claim_worker(&volume).is_none());
    assert!(stop
        .publish_index(&lifecycle.indices, index(&volume))
        .is_none());
}

#[test]
fn reenable_waits_for_both_worker_and_existing_leases() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let lease = lifecycle.acquire_volume('R').unwrap().unwrap();
    lifecycle.set_drive_indexing('R', false).unwrap();
    lifecycle.set_drive_indexing('R', true).unwrap();
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&volume));
    assert!(lifecycle.claim_worker(&volume).is_none());
    drop(ticket);
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&volume));
    assert!(lifecycle.claim_worker(&volume).is_none());
    while fixture.events.try_recv().is_ok() {}
    drop(lease);
    fixture
        .events
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&volume));
    assert!(lifecycle.claim_worker(&volume).is_some());
}

#[test]
fn physical_disconnect_cannot_bypass_a_toggle_drain() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.set_drive_indexing('R', false).unwrap();
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    lifecycle.set_drive_indexing('R', true).unwrap();
    assert!(lifecycle.claim_worker(&volume).is_none());
    drop(ticket);
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&volume));
    assert!(lifecycle.claim_worker(&volume).is_some());
}

#[test]
fn cancelled_pnp_removal_does_not_override_disabled_preference() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    drop(lifecycle.claim_worker(&volume).unwrap());
    assert!(lifecycle.prepare_remove_identity(&volume.volume_id, volume.volume_serial));
    lifecycle.set_drive_indexing('R', false).unwrap();
    lifecycle.removal_failed_identity(&volume.volume_id, volume.volume_serial);
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&volume));
    assert!(!lifecycle.is_indexing_enabled('R'));
    assert!(lifecycle.acquire_volume('R').is_err());
    assert!(lifecycle.claim_worker(&volume).is_none());
}

#[test]
fn failed_write_keeps_policy_and_worker_unchanged() {
    let fixture = Fixture::new();
    let ticket = fixture.lifecycle.claim_worker(&volume('R')).unwrap();
    fixture
        .connection()
        .execute_batch(
            "CREATE TRIGGER fail_config BEFORE INSERT ON drive_indexing_config
         BEGIN SELECT RAISE(ABORT, 'test write failure'); END;",
        )
        .unwrap();
    assert!(fixture.lifecycle.set_drive_indexing('R', false).is_err());
    assert!(fixture.lifecycle.is_indexing_enabled('R'));
    assert!(!ticket.stop_signal().is_stopped());
    assert!(fixture.db.load_drive_indexing_config().unwrap().is_empty());
}

#[test]
fn concurrent_edits_converge_to_the_persisted_policy() {
    let fixture = Fixture::new();
    std::thread::scope(|scope| {
        for enabled in [true, false, true, false] {
            let lifecycle = &fixture.lifecycle;
            scope.spawn(move || {
                for _ in 0..5 {
                    lifecycle.set_drive_indexing('r', enabled).unwrap();
                }
            });
        }
    });
    assert_eq!(
        fixture.lifecycle.is_indexing_enabled('R'),
        fixture.db.load_drive_indexing_config().unwrap()[&'R']
    );
}

#[test]
fn disable_keeps_records_and_filters_late_progress_from_status() {
    let fixture = Fixture::new();
    fixture
        .connection()
        .execute_batch(
            "INSERT INTO file_records (frn, drive_letter, name, parent_frn, is_dir)
         VALUES (10, 'R', 'file.txt', 5, 0);",
        )
        .unwrap();
    fixture.lifecycle.set_drive_indexing('R', false).unwrap();
    fixture
        .lifecycle
        .indexing_progress
        .set_scanning('R', 10, "persisting");
    let mut status = mtt_search_protocol::IndexStatusInfo {
        volumes: fixture.lifecycle.indexing_progress.snapshot(),
        total_files_indexed: 10,
        service_executable_path: String::new(),
    };
    fixture.lifecycle.filter_disabled_status(&mut status);
    assert!(status.volumes.is_empty());
    assert_eq!(status.total_files_indexed, 0);
    let count: i64 = fixture
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM file_records WHERE drive_letter = 'R'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn discovery_completion_is_explicit_and_disabled_inventory_is_not_expected() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture.lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: false,
            ..
        }
    ));
    fixture
        .lifecycle
        .record_drive_discovery(&[volume('R')], fixture.lifecycle.drive_config_revision());
    fixture.lifecycle.set_drive_indexing('R', false).unwrap();
    fixture
        .lifecycle
        .record_drive_discovery(&[], fixture.lifecycle.drive_config_revision());
    assert!(matches!(fixture.lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: true, enabled_service_volumes, ..
        } if enabled_service_volumes.is_empty()));
}

#[test]
fn a_discovery_pass_started_before_a_toggle_cannot_report_completion() {
    let fixture = Fixture::new();
    let revision = fixture.lifecycle.drive_config_revision();
    fixture.lifecycle.set_drive_indexing('R', false).unwrap();
    fixture.lifecycle.set_drive_indexing('R', true).unwrap();
    fixture.lifecycle.record_drive_discovery(&[], revision);
    assert!(matches!(
        fixture.lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: false,
            ..
        }
    ));
    fixture
        .lifecycle
        .record_drive_discovery(&[volume('R')], fixture.lifecycle.drive_config_revision());
    assert!(matches!(fixture.lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: true, enabled_service_volumes, ..
        } if enabled_service_volumes == vec!['R']));
}

#[test]
fn health_access_ignores_indexing_disable_but_respects_pnp_removal() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.set_drive_indexing('R', false).unwrap();
    assert!(lifecycle.acquire_volume('R').is_err());
    assert!(lifecycle.acquire_hardware_volume('R').unwrap().is_some());
    drop(ticket);
    assert!(lifecycle.prepare_remove_identity(&volume.volume_id, volume.volume_serial));
    assert!(lifecycle.acquire_hardware_volume('R').is_err());
    lifecycle.removal_failed_identity(&volume.volume_id, volume.volume_serial);
    assert!(lifecycle.acquire_hardware_volume('R').unwrap().is_some());
    assert!(lifecycle.acquire_volume('R').is_err());
}

#[test]
fn search_leases_and_activity_policy_share_one_configuration_snapshot() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    ticket
        .stop_signal()
        .publish_index(&lifecycle.indices, index(&volume))
        .unwrap();
    let before = lifecycle.acquire_search_access();
    lifecycle.set_drive_indexing('R', false).unwrap();
    let after = lifecycle.acquire_search_access();
    assert!(before.is_indexing_enabled('R'));
    assert!(before.find_handle('R').is_some());
    assert!(!after.is_indexing_enabled('r'));
    assert!(after.find_handle('R').is_none());
    assert!(after.is_indexing_enabled('S'));
}

#[test]
fn disconnect_before_a_toggle_also_retains_the_old_worker_until_exit() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    lifecycle.set_drive_indexing('R', false).unwrap();
    lifecycle.set_drive_indexing('R', true).unwrap();
    assert!(lifecycle.claim_worker(&volume).is_none());
    drop(ticket);
    assert!(lifecycle.claim_worker(&volume).is_some());
}

#[test]
fn retiring_a_suspended_runtime_requires_a_new_discovery_before_completion() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.set_drive_indexing('R', false).unwrap();
    lifecycle.set_drive_indexing('R', true).unwrap();
    let revision = lifecycle.drive_config_revision();
    drop(ticket);
    // This pass skipped the letter while the runtime was still suspended.
    lifecycle.reconcile_discovered_volumes(&[]);
    lifecycle.record_drive_discovery(&[], revision);
    assert!(matches!(
        lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: false,
            ..
        }
    ));
    lifecycle.record_drive_discovery(&[volume], lifecycle.drive_config_revision());
    assert!(matches!(lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: true, enabled_service_volumes, ..
        } if enabled_service_volumes == vec!['R']));
}
