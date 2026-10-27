use super::super::{tests::NativeWindow, TrayAction};
use super::*;
use windows::core::{Error, HRESULT, PCWSTR};
use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE};

fn reset() {
    STATE.with(|s| *s.borrow_mut() = State::default());
    while super::super::take_action().is_some() {}
}

fn ownership(label: &str) -> windows::core::Result<Ownership> {
    let name: Vec<u16> = format!("Local\\MTT.Tray.Test.{label}.{}", std::process::id())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    Ownership::new(PCWSTR(name.as_ptr()))
}

#[test]
fn failed_timer_disables_hiding_even_when_another_owner_exists() {
    reset();
    let local = NativeWindow::new();
    let other = NativeWindow::new();
    unsafe {
        SetPropW(
            other.0,
            w!("MTT.FileManager.TrayOwner"),
            Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
        )
        .unwrap();
    }
    assert!(!initialize_with(local.0, ownership("FailedTimer"), || {
        false
    }));
    assert!(!ready(local.0));
    assert!(!available(local.0));
    tick(local.0);
    STATE.with(|s| {
        let state = s.borrow();
        assert!(state.mutex.is_none());
        assert!(!state.owner && !state.timer_ready);
    });
    assert!(unsafe { GetPropW(local.0, w!("MTT.FileManager.TrayOwner")) }.is_invalid());
}

#[test]
fn failed_mutex_does_not_start_the_coordination_timer() {
    reset();
    let local = NativeWindow::new();
    let error = Error::new(HRESULT(0x80004005u32 as i32), "Simulated failure");
    assert!(!initialize_with(local.0, Err(error), || panic!(
        "timer must not start"
    )));
    assert!(!available(local.0));
}

#[test]
fn successful_initialization_requires_both_resources_and_shutdown_disables_it() {
    reset();
    let local = NativeWindow::new();
    assert!(initialize_with(local.0, ownership("Ready"), || true));
    assert!(ready(local.0));
    shutdown(local.0);
    assert!(!ready(local.0));
    assert!(!available(local.0));
}

#[test]
fn failed_close_post_keeps_the_window_discoverable_and_preserves_actions() {
    reset();
    let local = NativeWindow::new();
    let reference = WindowRef::capture(local.0).unwrap();
    super::super::queue_action(TrayAction::Open);
    assert!(!request_exit_with(local.0, || false));
    assert!(reference.is_current());
    assert!(!STATE.with(|s| s.borrow().exiting));
    assert_eq!(super::super::take_action(), Some(TrayAction::Open));
    assert_eq!(super::super::take_action(), None);
}

#[test]
fn acknowledged_exit_posts_close_and_only_marks_the_receiving_window() {
    reset();
    let local = NativeWindow::new();
    let unrelated = NativeWindow::new();
    assert!(request_exit(local.0));
    assert_eq!(super::super::take_action(), Some(TrayAction::Exit));
    assert!(WindowRef::capture(local.0).is_none());
    assert!(WindowRef::capture(unrelated.0).is_some());
    assert!(unsafe {
        PeekMessageW(
            &mut MSG::default(),
            Some(local.0),
            WM_CLOSE,
            WM_CLOSE,
            PM_REMOVE,
        )
    }
    .as_bool());
    assert!(request_exit_with(local.0, || panic!(
        "duplicate exit must be idempotent"
    )));
}
