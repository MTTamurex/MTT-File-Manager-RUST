use super::{normalize_volume_id, volume_ids_match, VolumeLifecycle, VolumeStopSignal};
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::DiscoveredVolume;
use crate::volume_indices;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

#[path = "regression_tests.rs"]
mod regressions;

fn volume() -> DiscoveredVolume {
    DiscoveredVolume {
        drive_letter: 'R',
        volume_id: r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".to_string(),
        volume_serial: 0x1234_abcd,
        label: String::new(),
        file_system: "NTFS".to_string(),
        usn_supported: true,
    }
}

#[test]
fn volume_identity_normalization_ignores_trailing_separator_and_case() {
    assert_eq!(
        normalize_volume_id(r"\\?\Volume{ABCDEF01-2345-6789-ABCD-EF0123456789}\"),
        normalize_volume_id(r"\\?\volume{abcdef01-2345-6789-abcd-ef0123456789}")
    );
}

#[test]
fn pnp_symbolic_link_matches_the_mounted_volume_guid() {
    assert!(volume_ids_match(
        r"\\?\Volume{ABCDEF01-2345-6789-ABCD-EF0123456789}\",
        r"\??\Volume{abcdef01-2345-6789-abcd-ef0123456789}"
    ));
    assert!(!volume_ids_match(
        r"\\?\Volume{ABCDEF01-2345-6789-ABCD-EF0123456789}\",
        r"\\?\Volume{FEDCBA98-7654-3210-FEDC-BA9876543210}\"
    ));
}

#[test]
fn local_stop_wakes_a_waiting_indexer_immediately() {
    let stop = VolumeStopSignal::new(Arc::new(AtomicBool::new(false)));
    let waiter = stop.clone();
    let thread = std::thread::spawn(move || waiter.wait_or_stop(Duration::from_secs(30)));

    std::thread::sleep(Duration::from_millis(10));
    stop.request_stop();

    assert!(thread.join().unwrap());
}

#[test]
fn removal_waits_for_io_leases_and_the_volume_worker() {
    let (wake_tx, _wake_rx) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake_tx,
    );
    let volume = volume();
    let ticket = lifecycle
        .claim_worker(&volume)
        .expect("mounted volume should claim an index worker");
    let stop = ticket.stop_signal();
    let io_lease = lifecycle
        .acquire_volume(volume.drive_letter)
        .unwrap()
        .expect("mounted volume should provide an I/O lease");

    let lifecycle_for_removal = lifecycle.clone();
    let volume_id = volume.volume_id.clone();
    let volume_serial = volume.volume_serial;
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let remover = std::thread::spawn(move || {
        let _ = result_tx
            .send(lifecycle_for_removal.prepare_remove_identity(&volume_id, volume_serial));
    });

    let deadline = Instant::now() + Duration::from_secs(1);
    while !stop.is_stopped() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        stop.is_stopped(),
        "the removal request must cancel indexing"
    );
    assert!(result_rx.recv_timeout(Duration::from_millis(30)).is_err());

    drop(io_lease);
    drop(ticket);
    assert!(result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    remover.join().unwrap();
    assert!(lifecycle.acquire_volume(volume.drive_letter).is_err());

    lifecycle.removal_failed_identity(&volume.volume_id, volume.volume_serial);
    assert!(lifecycle
        .acquire_volume(volume.drive_letter)
        .unwrap()
        .is_some());
}

#[test]
fn maintenance_survives_discovery_without_reopening_the_volume() {
    let (wake, _events) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake,
    );
    let volume = volume();
    drop(lifecycle.claim_worker(&volume).unwrap());
    assert!(lifecycle.prepare_maintenance_identity(&volume.volume_id, volume.volume_serial));

    // Discovery intentionally omits suspended drives. Their suspension must
    // not be interpreted as physical unplugging and discarded.
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(lifecycle.is_drive_suspended(volume.drive_letter));
    assert!(lifecycle.acquire_volume(volume.drive_letter).is_err());
    assert!(lifecycle.claim_worker(&volume).is_none());

    lifecycle.removal_failed_identity(&volume.volume_id, volume.volume_serial);
    assert!(!lifecycle.is_drive_suspended(volume.drive_letter));
    assert!(lifecycle.claim_worker(&volume).is_some());
}

#[test]
fn maintenance_timeout_does_not_allow_new_io_or_indexers() {
    let (wake, _events) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake,
    );
    let volume = volume();
    drop(lifecycle.claim_worker(&volume).unwrap());
    let lease = lifecycle.acquire_volume(volume.drive_letter).unwrap();
    assert!(!lifecycle.prepare_maintenance_identity(&volume.volume_id, volume.volume_serial));
    assert!(lifecycle.is_drive_suspended(volume.drive_letter));
    assert!(lifecycle.acquire_volume(volume.drive_letter).is_err());
    assert!(lifecycle.claim_worker(&volume).is_none());
    drop(lease);
    lifecycle.removal_failed_identity(&volume.volume_id, volume.volume_serial);
    assert!(lifecycle.claim_worker(&volume).is_some());
}

#[test]
fn reformatted_media_with_same_volume_guid_gets_a_new_runtime() {
    let (wake_tx, _wake_rx) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake_tx,
    );
    let original = volume();
    drop(lifecycle.claim_worker(&original).unwrap());

    let reformatted = DiscoveredVolume {
        volume_serial: 0x8765_4321,
        ..original.clone()
    };
    let ticket = lifecycle
        .claim_worker(&reformatted)
        .expect("changed filesystem serial should restart indexing");

    assert!(lifecycle
        .runtime_for_identity(&original.volume_id, original.volume_serial)
        .is_none());
    assert!(lifecycle
        .runtime_for_identity(&reformatted.volume_id, reformatted.volume_serial)
        .is_some());
    drop(ticket);
}

#[test]
fn stale_removal_notification_cannot_disconnect_reformatted_runtime() {
    let (wake_tx, _wake_rx) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake_tx,
    );
    let original = volume();
    drop(lifecycle.claim_worker(&original).unwrap());

    let reformatted = DiscoveredVolume {
        volume_serial: 0x8765_4321,
        ..original.clone()
    };
    drop(lifecycle.claim_worker(&reformatted).unwrap());

    lifecycle.disconnect_by_identity(&original.volume_id, original.volume_serial);

    assert!(lifecycle
        .runtime_for_identity(&original.volume_id, original.volume_serial)
        .is_none());
    assert!(lifecycle
        .runtime_for_identity(&reformatted.volume_id, reformatted.volume_serial)
        .is_some());
    assert!(lifecycle
        .acquire_volume(reformatted.drive_letter)
        .unwrap()
        .is_some());
}
