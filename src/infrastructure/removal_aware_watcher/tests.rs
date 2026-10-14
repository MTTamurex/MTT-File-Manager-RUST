use super::*;
use crate::infrastructure::windows::OwnedHandle;
use notify::windows::ReadDirectoryChangesWatcher;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_NONE, OPEN_EXISTING,
};

static TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn group(
    path: &Path,
    letter: char,
) -> (
    Group,
    crossbeam_channel::Receiver<notify::Result<notify::Event>>,
) {
    let (events_tx, events_rx) = crossbeam_channel::unbounded();
    let (completions_tx, completions_rx) = crossbeam_channel::unbounded();
    let handler = Arc::new(Mutex::new(move |event| {
        let _ = events_tx.send(event);
    }));
    let mut watcher = ReadDirectoryChangesWatcher::create(handler, completions_tx).unwrap();
    watcher.watch(path, RecursiveMode::NonRecursive).unwrap();
    let mut native = lifecycle::NativeState::new(watcher, completions_rx);
    native.paths.insert(path.to_path_buf());
    let control = Arc::new(lifecycle::Control {
        native: parking_lot::Mutex::new(native),
        monitor: parking_lot::Mutex::new(None),
    });
    lifecycle::register(letter, &control);
    (
        Group {
            registration: None,
            control,
        },
        events_rx,
    )
}

fn exclusive_directory_handle(path: &Path) -> Option<OwnedHandle> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .ok()
    .and_then(OwnedHandle::new)
}

#[test]
fn query_remove_closes_directory_handles_before_granting_and_preserves_other_drive() {
    let _test = TEST_LOCK.lock();
    lifecycle::resume('Z');
    lifecycle::resume('Y');
    let target = tempfile::tempdir().unwrap();
    let child = target.path().join("Folder");
    std::fs::create_dir(&child).unwrap();
    let other = tempfile::tempdir().unwrap();
    let (target_group, _target_events) = group(target.path(), 'Z');
    let (other_group, other_events) = group(other.path(), 'Y');
    let mut watcher = RemovalAwareWatcher::new(|_: notify::Result<notify::Event>| {}).unwrap();
    let control = target_group.control.clone();
    {
        let mut native = control.native.lock();
        native
            .watcher
            .as_mut()
            .unwrap()
            .watch(&child, RecursiveMode::NonRecursive)
            .unwrap();
        native.paths.insert(child.clone());
    }
    watcher.groups.insert(Some('Z'), target_group);
    watcher.groups.insert(Some('Y'), other_group);
    assert!(exclusive_directory_handle(target.path()).is_none());

    assert_eq!(
        notifications::simulate_notification(
            'Z',
            control.clone(),
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVE,
            None
        ),
        ERROR_SUCCESS.0
    );
    assert!(is_suspended('Z'));
    assert!(
        exclusive_directory_handle(target.path()).is_some(),
        "Grant requires actual directory-handle closure"
    );
    assert!(
        exclusive_directory_handle(&child).is_some(),
        "All Miller ancestor/child handles must close"
    );
    assert!(exclusive_directory_handle(other.path()).is_none());
    assert!(!watcher.needs_refresh());
    assert!(
        watcher
            .watch(Path::new(r"Z:\Folder"), RecursiveMode::NonRecursive)
            .is_err(),
        "Late setups cannot reopen a suspended drive"
    );

    std::fs::write(other.path().join("file.txt"), b"change").unwrap();
    assert!(other_events
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .is_ok());

    assert_eq!(
        notifications::simulate_notification(
            'Z',
            control,
            CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED,
            None
        ),
        ERROR_SUCCESS.0
    );
    assert!(!is_suspended('Z'));
    assert!(
        watcher.needs_refresh(),
        "Cancelled eject must trigger native watcher recreation"
    );
    drop(watcher);
}

#[test]
fn removal_during_setup_obeys_deadline_and_blocks_new_watches() {
    let _test = TEST_LOCK.lock();
    lifecycle::resume('Z');
    let directory = tempfile::tempdir().unwrap();
    let (group, _events) = group(directory.path(), 'Z');
    let setup_guard = group.control.native.lock();
    let deadline = Instant::now() + Duration::from_millis(30);
    assert!(!lifecycle::suspend('Z', deadline));
    assert!(is_suspended('Z'));
    drop(setup_guard);
    assert!(lifecycle::suspend(
        'Z',
        Instant::now() + Duration::from_secs(2)
    ));
    assert!(exclusive_directory_handle(directory.path()).is_some());
    lifecycle::resume('Z');
    drop(group);
}

#[test]
fn volume_lock_releases_watcher_and_unlock_allows_recreation() {
    use windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_ACTION_DEVICECUSTOMEVENT;
    use windows::Win32::UI::WindowsAndMessaging::{GUID_IO_VOLUME_LOCK, GUID_IO_VOLUME_UNLOCK};
    let _test = TEST_LOCK.lock();
    lifecycle::resume('Z');
    let directory = tempfile::tempdir().unwrap();
    let (group, _events) = group(directory.path(), 'Z');
    let control = group.control.clone();
    let mut watcher = RemovalAwareWatcher::new(|_: notify::Result<notify::Event>| {}).unwrap();
    watcher.groups.insert(Some('Z'), group);

    assert_eq!(
        notifications::simulate_notification(
            'Z',
            control.clone(),
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT,
            Some(GUID_IO_VOLUME_LOCK)
        ),
        ERROR_SUCCESS.0
    );
    assert!(is_suspended('Z'));
    assert!(exclusive_directory_handle(directory.path()).is_some());
    assert!(!watcher.needs_refresh());
    assert_eq!(
        notifications::simulate_notification(
            'Z',
            control,
            CM_NOTIFY_ACTION_DEVICECUSTOMEVENT,
            Some(GUID_IO_VOLUME_UNLOCK)
        ),
        ERROR_SUCCESS.0
    );
    assert!(!is_suspended('Z'));
    assert!(watcher.needs_refresh());
    drop(watcher);
}
