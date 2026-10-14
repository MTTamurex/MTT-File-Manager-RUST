use super::*;
use crate::indexing_progress::IndexingProgress;
use crate::usn_journal::DiscoveredVolume;
use crate::volume_indices;
use std::sync::mpsc;
use std::time::Duration;
use windows::Win32::UI::WindowsAndMessaging::{
    GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_LOCK, GUID_IO_VOLUME_MOUNT,
};

#[test]
fn handle_query_remove_closes_registration_handle_and_stops_indexer() {
    let (wake, _events) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake,
    );
    let volume = DiscoveredVolume {
        drive_letter: 'R',
        volume_id: r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".to_string(),
        volume_serial: 0x1234_abcd,
        label: String::new(),
        file_system: "exFAT".to_string(),
        usn_supported: false,
    };
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let stop = ticket.stop_signal();
    let worker = std::thread::spawn(move || {
        while !stop.is_stopped() {
            stop.wait_or_stop(Duration::from_millis(10));
        }
        drop(ticket);
    });
    let handle = unsafe {
        windows::Win32::System::Threading::CreateEventW(None, true, false, None).unwrap()
    };
    let context = Context {
        lifecycle: lifecycle.clone(),
        drive_letter: volume.drive_letter,
        volume_id: volume.volume_id.clone(),
        volume_serial: volume.volume_serial,
        handle: Mutex::new(Some(handle.0 as usize)),
        rearm: AtomicBool::new(false),
    };
    let result = unsafe {
        callback(
            HCMNOTIFICATION::default(),
            (&context as *const Context).cast(),
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVE,
            std::ptr::null(),
            0,
        )
    };
    assert_eq!(result, ERROR_SUCCESS.0);
    assert!(context.handle.lock().is_none());
    assert!(lifecycle.acquire_volume('R').is_err());
    worker.join().unwrap();

    let result = unsafe {
        callback(
            HCMNOTIFICATION::default(),
            (&context as *const Context).cast(),
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
            std::ptr::null(),
            0,
        )
    };
    assert_eq!(result, ERROR_SUCCESS.0);
    assert!(context.rearm.load(Ordering::Acquire));
    assert!(lifecycle.acquire_volume('R').is_ok());
}

#[test]
fn volume_lock_and_unlock_events_release_and_rearm_service_access() {
    let (wake, _events) = mpsc::channel();
    let lifecycle = VolumeLifecycle::new(
        Arc::new(AtomicBool::new(false)),
        volume_indices::new_shared(),
        Arc::new(IndexingProgress::new()),
        wake,
    );
    let volume = DiscoveredVolume {
        drive_letter: 'R',
        volume_id: r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".to_string(),
        volume_serial: 0x1234_abcd,
        label: String::new(),
        file_system: "exFAT".to_string(),
        usn_supported: false,
    };
    let ticket = lifecycle.claim_worker(&volume).unwrap();
    let stop = ticket.stop_signal();
    let worker = std::thread::spawn(move || {
        while !stop.is_stopped() {
            stop.wait_or_stop(Duration::from_millis(10));
        }
        drop(ticket);
    });
    let handle = unsafe {
        windows::Win32::System::Threading::CreateEventW(None, true, false, None).unwrap()
    };
    let context = Context {
        lifecycle: lifecycle.clone(),
        drive_letter: volume.drive_letter,
        volume_id: volume.volume_id.clone(),
        volume_serial: volume.volume_serial,
        handle: Mutex::new(Some(handle.0 as usize)),
        rearm: AtomicBool::new(false),
    };
    let mut event = CM_NOTIFY_EVENT_DATA {
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
        ..Default::default()
    };
    event.u = CM_NOTIFY_EVENT_DATA_0 {
        DeviceHandle: CM_NOTIFY_EVENT_DATA_0_1 {
            EventGuid: GUID_IO_VOLUME_LOCK,
            ..Default::default()
        },
    };

    let result = unsafe {
        callback(
            HCMNOTIFICATION::default(),
            (&context as *const Context).cast(),
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT,
            &event,
            std::mem::size_of::<CM_NOTIFY_EVENT_DATA>() as u32,
        )
    };
    assert_eq!(result, ERROR_SUCCESS.0);
    assert!(context.handle.lock().is_none());
    assert!(lifecycle.acquire_volume('R').is_err());
    worker.join().unwrap();

    // A dismount removes the old runtime so no index can be reused while
    // the filesystem is being replaced.
    event.u = CM_NOTIFY_EVENT_DATA_0 {
        DeviceHandle: CM_NOTIFY_EVENT_DATA_0_1 {
            EventGuid: GUID_IO_VOLUME_DISMOUNT,
            ..Default::default()
        },
    };
    unsafe {
        callback(
            HCMNOTIFICATION::default(),
            (&context as *const Context).cast(),
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT,
            &event,
            std::mem::size_of::<CM_NOTIFY_EVENT_DATA>() as u32,
        );
    }
    lifecycle.reconcile_discovered_volumes(&[]);
    assert!(lifecycle.acquire_volume('R').unwrap().is_none());

    let reformatted = DiscoveredVolume {
        volume_serial: 0x8765_4321,
        ..volume.clone()
    };
    lifecycle.reconcile_discovered_volumes(std::slice::from_ref(&reformatted));
    event.u = CM_NOTIFY_EVENT_DATA_0 {
        DeviceHandle: CM_NOTIFY_EVENT_DATA_0_1 {
            EventGuid: GUID_IO_VOLUME_MOUNT,
            ..Default::default()
        },
    };
    let result = unsafe {
        callback(
            HCMNOTIFICATION::default(),
            (&context as *const Context).cast(),
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT,
            &event,
            std::mem::size_of::<CM_NOTIFY_EVENT_DATA>() as u32,
        )
    };
    assert_eq!(result, ERROR_SUCCESS.0);
    assert!(context.rearm.load(Ordering::Acquire));
    assert!(lifecycle.claim_worker(&reformatted).is_some());
}
