use super::*;
use crate::file_index::{IndexState, VolumeIndex};

fn setup() -> (Arc<VolumeLifecycle>, mpsc::Receiver<()>) {
    let (wake, events) = mpsc::channel();
    (
        VolumeLifecycle::new(
            Arc::new(AtomicBool::new(false)),
            volume_indices::new_shared(),
            Arc::new(IndexingProgress::new()),
            wake,
        ),
        events,
    )
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
fn identity_timeout_retries_after_worker_exit_without_reopening_old_identity() {
    let (lifecycle, events) = setup();
    let original = volume();
    let ticket = lifecycle.claim_worker(&original).unwrap();
    let replacement = DiscoveredVolume {
        volume_serial: original.volume_serial + 1,
        ..original.clone()
    };
    assert!(lifecycle
        .ensure_runtime_until(&replacement, Instant::now())
        .is_none());
    assert!(ticket.stop_signal().is_stopped());
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(lifecycle.is_drive_suspended(original.drive_letter));
    lifecycle.removal_failed_identity(&original.volume_id, original.volume_serial);
    assert!(lifecycle.acquire_volume(original.drive_letter).is_err());
    drop(ticket);
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(!lifecycle.is_drive_suspended(original.drive_letter));
    assert!(events.try_recv().is_ok());
    assert!(lifecycle.claim_worker(&replacement).is_some());
    assert!(lifecycle
        .runtime_for_identity(&original.volume_id, original.volume_serial)
        .is_none());
}

#[test]
fn identity_timeout_retries_after_last_io_lease_is_released() {
    let (lifecycle, events) = setup();
    let original = volume();
    drop(lifecycle.claim_worker(&original).unwrap());
    let lease = lifecycle.acquire_volume(original.drive_letter).unwrap();
    let replacement = DiscoveredVolume {
        volume_serial: original.volume_serial + 1,
        ..original.clone()
    };
    assert!(lifecycle
        .ensure_runtime_until(&replacement, Instant::now())
        .is_none());
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(lifecycle.is_drive_suspended(original.drive_letter));
    drop(lease);
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(!lifecycle.is_drive_suspended(original.drive_letter));
    assert!(events.try_recv().is_ok());
    assert!(lifecycle.claim_worker(&replacement).is_some());
}

#[test]
fn replacement_cleanup_does_not_release_a_maintenance_suspension() {
    let (lifecycle, _) = setup();
    let original = volume();
    drop(lifecycle.claim_worker(&original).unwrap());
    assert!(lifecycle.prepare_maintenance_identity(&original.volume_id, original.volume_serial));
    let replacement = DiscoveredVolume {
        volume_serial: original.volume_serial + 1,
        ..original.clone()
    };
    assert!(lifecycle
        .ensure_runtime_until(&replacement, Instant::now())
        .is_none());
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(lifecycle.is_drive_suspended(original.drive_letter));
    assert!(lifecycle
        .runtime_for_identity(&original.volume_id, original.volume_serial)
        .is_some());
    lifecycle.removal_failed_identity(&original.volume_id, original.volume_serial);
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(lifecycle.claim_worker(&replacement).is_some());
}

#[test]
fn worker_past_its_last_stop_check_cannot_publish_after_disconnect() {
    let (lifecycle, _) = setup();
    let volume = volume();
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let stop = ticket.stop_signal();
    let indices = lifecycle.indices.clone();
    let to_publish = index(&volume);
    let (checked_tx, checked_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        assert!(!stop.is_stopped());
        checked_tx.send(()).unwrap();
        resume_rx.recv().unwrap();
        stop.publish_index(&indices, to_publish).is_some()
    });
    checked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    resume_tx.send(()).unwrap();
    assert!(!worker.join().unwrap());
    assert!(volume_indices::snapshot_handles(&lifecycle.indices).is_empty());
    drop(ticket);
}

#[test]
fn disconnect_waits_for_inflight_publication_then_removes_it() {
    let (lifecycle, _) = setup();
    let volume = volume();
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let stop = ticket.stop_signal();
    let indices = lifecycle.indices.clone();
    let to_publish = index(&volume);
    let blocked_indices = lifecycle.indices.write();
    let publisher = std::thread::spawn(move || stop.publish_index(&indices, to_publish).is_some());
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if lifecycle.runtimes.try_lock().is_none() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "publisher did not acquire lifecycle lock"
        );
        std::thread::yield_now();
    }
    let remover_lifecycle = lifecycle.clone();
    let remover = std::thread::spawn(move || {
        remover_lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    });
    drop(blocked_indices);
    assert!(publisher.join().unwrap());
    remover.join().unwrap();
    assert!(volume_indices::snapshot_handles(&lifecycle.indices).is_empty());
    drop(ticket);
}

#[test]
fn stale_generation_and_reconnected_runtime_cannot_publish() {
    let (lifecycle, _) = setup();
    let volume = volume();
    let original = lifecycle.claim_worker(&volume).unwrap();
    let stale = original.stop_signal();
    drop(original);
    let current = lifecycle.claim_worker(&volume).unwrap();
    assert!(stale
        .publish_index(&lifecycle.indices, index(&volume))
        .is_none());
    assert!(current
        .stop_signal()
        .publish_index(&lifecycle.indices, index(&volume))
        .is_some());
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    let reconnected = lifecycle.claim_worker(&volume).unwrap();
    assert!(current
        .stop_signal()
        .publish_index(&lifecycle.indices, index(&volume))
        .is_none());
    assert!(reconnected
        .stop_signal()
        .publish_index(&lifecycle.indices, index(&volume))
        .is_some());
}

#[test]
fn search_excludes_suspended_and_orphan_indices_but_keeps_other_volumes() {
    let (lifecycle, _) = setup();
    let available = volume();
    let suspended = DiscoveredVolume {
        drive_letter: 'S',
        volume_id: "other-volume".into(),
        ..available.clone()
    };
    for volume in [&available, &suspended] {
        let ticket = lifecycle.claim_worker(volume).unwrap();
        assert!(ticket
            .stop_signal()
            .publish_index(&lifecycle.indices, index(volume))
            .is_some());
    }
    assert!(lifecycle.prepare_maintenance_identity(&suspended.volume_id, suspended.volume_serial));
    let orphan = DiscoveredVolume {
        drive_letter: 'T',
        ..available.clone()
    };
    volume_indices::upsert(&lifecycle.indices, index(&orphan));
    let access = lifecycle.acquire_search_access();
    assert!(access.find_handle('R').is_some());
    assert!(access.find_handle('S').is_none());
    assert!(access.find_handle('T').is_none());
    let page = crate::file_index::search_page(access.handles(), "file", 0, 10);
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].full_path.starts_with("R:"));
    let runtime = lifecycle
        .runtime_for_identity(&available.volume_id, available.volume_serial)
        .unwrap();
    runtime.usage.begin_removal();
    assert!(!runtime.usage.wait_until_idle(Instant::now()));
    drop(access);
    assert!(runtime.usage.wait_until_idle(Instant::now()));
    assert!(lifecycle.acquire_search_access().handles().is_empty());
}
