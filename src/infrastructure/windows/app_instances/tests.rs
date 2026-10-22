use super::*;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn multiple_instances_require_explicit_opt_in() {
    for value in [None, Some("false"), Some(""), Some("invalid"), Some("TRUE")] {
        assert!(!allows_multiple_instances(value));
    }
    assert!(allows_multiple_instances(Some("true")));
}

#[test]
fn concurrent_startup_waits_until_the_receiver_is_ready() {
    let name: Vec<u16> = format!("Local\\MTT.FileManager.Test.Startup.{}", std::process::id())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let first = StartupLock::acquire_named(PCWSTR(name.as_ptr())).unwrap();
    let (attempting_tx, attempting_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let second = std::thread::spawn(move || {
        attempting_tx.send(()).unwrap();
        let _lock = StartupLock::acquire_named(PCWSTR(name.as_ptr())).unwrap();
        acquired_tx.send(()).unwrap();
    });
    attempting_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        acquired_rx.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    first.hold_until_window_ready();
    finish_startup(
        || true,
        || {
            assert!(matches!(
                acquired_rx.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            Ok(())
        },
    )
    .unwrap();
    acquired_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    second.join().unwrap();
}

#[test]
fn instance_policy_changes_are_visible_without_session_save_or_restart() {
    use crate::infrastructure::app_state_db::AppStateDb;
    let dir = tempfile::tempdir().unwrap();
    let first = AppStateDb::new(dir.path().to_path_buf()).unwrap();
    let second = AppStateDb::new(dir.path().to_path_buf()).unwrap();
    let read = || {
        allows_multiple_instances(
            second
                .get_preference(ALLOW_MULTIPLE_INSTANCES_KEY)
                .as_deref(),
        )
    };
    assert!(!read());
    first
        .set_preference(ALLOW_MULTIPLE_INSTANCES_KEY, "true")
        .unwrap();
    assert!(read());
    first
        .set_preference(ALLOW_MULTIPLE_INSTANCES_KEY, "false")
        .unwrap();
    assert!(!read());
}

#[test]
fn native_restore_wakes_hidden_and_minimized_windows() {
    use crate::infrastructure::windows::{system_tray, window_subclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, IsIconic, IsWindowVisible, ShowWindow, SW_MINIMIZE,
        WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    };

    struct TestWindow(HWND);
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    let window = TestWindow(unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Instance restore test"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            320,
            240,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    let hwnd = window.0;
    assert!(window_subclass::install_borderless_subclass(hwnd));
    register_main_window(hwnd).unwrap();
    assert!(!unsafe { GetPropW(hwnd, w!("MTT.FileManager.MainWindow")) }.is_invalid());
    assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());

    assert!(restore_main_window(hwnd));
    assert!(unsafe { IsWindowVisible(hwnd) }.as_bool());
    assert_eq!(
        system_tray::take_action(),
        Some(system_tray::TrayAction::Open)
    );

    unsafe {
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
    }
    assert!(unsafe { IsIconic(hwnd) }.as_bool());
    assert!(restore_main_window(hwnd));
    assert!(!unsafe { IsIconic(hwnd) }.as_bool());
    assert_eq!(
        system_tray::take_action(),
        Some(system_tray::TrayAction::Open)
    );

    system_tray::hide_window(hwnd);
    assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
    assert!(restore_main_window(hwnd));
    assert!(unsafe { IsWindowVisible(hwnd) }.as_bool());
    assert_eq!(
        system_tray::take_action(),
        Some(system_tray::TrayAction::Open)
    );
}

#[test]
fn failed_startup_releases_the_mutex_without_publishing_a_window() {
    for (stage, receiver_installed) in [("receiver", false), ("registration", true)] {
        let name: Vec<u16> = format!(
            "Local\\MTT.FileManager.Test.FailedStartup.{}.{}",
            std::process::id(),
            stage
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        StartupLock::acquire_named(PCWSTR(name.as_ptr()))
            .unwrap()
            .hold_until_window_ready();
        let mut registration_attempted = false;
        let result = finish_startup(
            || receiver_installed,
            || {
                registration_attempted = true;
                // An invalid window deterministically fails SetPropW.
                register_main_window(HWND::default())
            },
        );
        assert!(result.is_err());
        assert_eq!(registration_attempted, receiver_installed);
        STARTUP_LOCK.with(|lock| assert!(lock.borrow().is_none()));
        let (tx, rx) = mpsc::channel();
        let next = std::thread::spawn(move || {
            let _lock = StartupLock::acquire_named(PCWSTR(name.as_ptr())).unwrap();
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        next.join().unwrap();
    }
}
