use super::*;
use std::time::Duration;

#[test]
fn disconnected_work_remains_visible_to_pnp_and_blocks_new_hardware_io() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);

    assert!(lifecycle.is_drive_suspended('R'));
    assert!(lifecycle.is_removing(&volume.volume_id, volume.volume_serial));
    assert!(lifecycle.acquire_hardware_volume('R').is_err());
    assert!(lifecycle.acquire_volume('R').is_err());
    drop(ticket);
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(!lifecycle.is_drive_suspended('R'));
}

#[test]
fn pnp_removal_must_wait_for_a_worker_already_in_the_disconnected_drain() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    let (result, received) = mpsc::channel();
    let other = lifecycle.clone();
    let remover = std::thread::spawn(move || {
        result
            .send(other.prepare_remove_identity(&volume.volume_id, volume.volume_serial))
            .unwrap();
    });
    let waited = received.recv_timeout(Duration::from_millis(50)).is_err();
    drop(ticket);
    remover.join().unwrap();
    assert!(
        waited,
        "disconnected work must not be reported as already drained"
    );
    assert!(received.recv_timeout(Duration::from_secs(1)).unwrap());
}

#[test]
fn stale_discovery_cannot_create_a_worker_after_a_disable_enable_cycle() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let revision = lifecycle.drive_config_revision();
    lifecycle.set_drive_indexing('R', false).unwrap();
    lifecycle.set_drive_indexing('R', true).unwrap();
    assert!(!lifecycle.reconcile_drive_discovery(std::slice::from_ref(&volume), revision));
    assert!(lifecycle
        .claim_discovered_worker(&volume, revision)
        .is_none());
    assert!(lifecycle.runtimes.lock().is_empty());
    assert!(!lifecycle.record_drive_discovery(std::slice::from_ref(&volume), revision));
    assert!(lifecycle
        .claim_discovered_worker(&volume, lifecycle.drive_config_revision())
        .is_some());
}

#[test]
fn stale_empty_discovery_does_not_disconnect_a_current_worker() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let revision = lifecycle.drive_config_revision();
    lifecycle.set_drive_indexing('S', false).unwrap();
    assert!(!lifecycle.reconcile_drive_discovery(&[], revision));
    assert!(!ticket.stop_signal().is_stopped());
    assert!(lifecycle.acquire_volume('R').unwrap().is_some());
}

#[test]
fn pnp_disconnect_invalidates_both_coverage_and_old_worker_admission() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let revision = lifecycle.drive_config_revision();
    assert!(lifecycle.record_drive_discovery(std::slice::from_ref(&volume), revision));
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    assert!(matches!(
        lifecycle.drive_indexing_config_response(),
        SearchResponse::DriveIndexingConfig {
            service_discovery_complete: false,
            ..
        }
    ));
    drop(ticket);
    assert!(lifecycle
        .claim_discovered_worker(&volume, revision)
        .is_none());
    assert!(!lifecycle.record_drive_discovery(&[], revision));
    assert!(lifecycle.runtimes.lock().is_empty());
}

#[test]
fn one_discovery_retires_all_missing_volumes_before_invalidating_its_revision() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let first = lifecycle.claim_worker(&volume('R')).unwrap();
    let second = lifecycle.claim_worker(&volume('S')).unwrap();
    assert!(!lifecycle.reconcile_drive_discovery(&[], lifecycle.drive_config_revision()));
    assert!(first.stop_signal().is_stopped());
    assert!(second.stop_signal().is_stopped());
    assert!(lifecycle.runtimes.lock().is_empty());
}

#[test]
fn retirement_of_a_disconnected_drain_requires_fresh_discovery() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    lifecycle.disconnect_by_identity(&volume.volume_id, volume.volume_serial);
    let revision = lifecycle.drive_config_revision();
    drop(ticket);
    assert!(!lifecycle.reconcile_drive_discovery(&[], revision));
    assert!(!lifecycle.record_drive_discovery(&[], revision));
    assert!(lifecycle.record_drive_discovery(&[], lifecycle.drive_config_revision()));
}

#[test]
fn pnp_suspension_pins_the_runtime_registry_before_waiting_for_its_worker_lock() {
    let fixture = Fixture::new();
    let lifecycle = &fixture.lifecycle;
    let volume = volume('R');
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let runtime = lifecycle
        .runtime_for_identity(&volume.volume_id, volume.volume_serial)
        .unwrap();
    let worker_lock = runtime.worker.lock();
    let (started, ready) = mpsc::channel();
    let other = lifecycle.clone();
    let remover = std::thread::spawn(move || {
        started.send(()).unwrap();
        other.prepare_remove_identity(&volume.volume_id, volume.volume_serial)
    });
    ready.recv_timeout(Duration::from_secs(1)).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let pinned = loop {
        if lifecycle.runtimes.try_lock().is_none() {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        std::thread::yield_now();
    };
    drop(worker_lock);
    drop(ticket);
    assert!(remover.join().unwrap());
    assert!(
        pinned,
        "a runtime must not be retired between PnP lookup and suspension"
    );
}
