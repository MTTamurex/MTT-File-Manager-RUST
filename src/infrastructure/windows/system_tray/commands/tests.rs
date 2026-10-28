use super::super::tests::NativeWindow;
use super::*;

#[test]
fn global_exit_closes_the_tray_owner_last_without_premarking_peers() {
    let owner = NativeWindow::new();
    let first = NativeWindow::new();
    let second = NativeWindow::new();
    let owner = WindowRef::capture(owner.0).unwrap();
    let first = WindowRef::capture(first.0).unwrap();
    let second = WindowRef::capture(second.0).unwrap();
    let mut sent = Vec::new();
    assert!(exit_windows(owner, &[owner, first, second], |window| {
        assert!(owner.is_current() && first.is_current() && second.is_current());
        sent.push(window);
        true
    }));
    assert_eq!(sent, [first, second, owner]);
}

#[test]
fn failed_global_exit_preserves_the_owner_and_the_failed_window() {
    let owner = NativeWindow::new();
    let first = NativeWindow::new();
    let failed = NativeWindow::new();
    let unattempted = NativeWindow::new();
    let owner = WindowRef::capture(owner.0).unwrap();
    let first = WindowRef::capture(first.0).unwrap();
    let failed = WindowRef::capture(failed.0).unwrap();
    let unattempted = WindowRef::capture(unattempted.0).unwrap();
    let mut sent = Vec::new();
    assert!(!exit_windows(
        owner,
        &[owner, first, failed, unattempted],
        |window| {
            sent.push(window);
            window != failed
        }
    ));
    assert_eq!(sent, [first, failed]);
    assert!(owner.is_current() && failed.is_current() && unattempted.is_current());
}

#[test]
fn already_closed_peers_do_not_block_global_exit() {
    let owner = NativeWindow::new();
    let peer = NativeWindow::new();
    let owner = WindowRef::capture(owner.0).unwrap();
    let peer_ref = WindowRef::capture(peer.0).unwrap();
    drop(peer);
    let mut sent = Vec::new();
    assert!(exit_windows(owner, &[peer_ref, owner], |window| {
        sent.push(window);
        true
    }));
    assert_eq!(sent, [owner]);
}

#[test]
fn a_peer_that_disappears_during_a_failed_send_does_not_block_global_exit() {
    let owner_window = NativeWindow::new();
    let owner = WindowRef::capture(owner_window.0).unwrap();
    let mut peer = Some(NativeWindow::new());
    let peer_ref = WindowRef::capture(peer.as_ref().unwrap().0).unwrap();
    let mut sent = Vec::new();
    assert!(exit_windows(owner, &[peer_ref, owner], |window| {
        sent.push(window);
        if window == peer_ref {
            drop(peer.take());
            false
        } else {
            true
        }
    }));
    assert_eq!(sent, [peer_ref, owner]);
}

#[test]
fn failed_individual_exit_does_not_mark_any_window_as_exiting() {
    let sender = NativeWindow::new();
    // STATIC does not implement our exit receiver, so it cannot acknowledge.
    let peer = NativeWindow::new();
    let peer = WindowRef::capture(peer.0).unwrap();
    assert!(!exit_window(sender.0, peer));
    assert!(WindowRef::capture(sender.0).is_some() && peer.is_current());
}

#[test]
fn local_activation_reports_a_rejected_queue_instead_of_restoring() {
    let native = NativeWindow::new();
    let window = WindowRef::capture(native.0).unwrap();
    while super::super::take_action().is_some() {}
    super::super::queue_action(TrayAction::Exit);
    assert!(!activate(
        native.0,
        window,
        Target {
            workspace: 1,
            panel: None
        }
    ));
    assert_eq!(super::super::take_action(), Some(TrayAction::Exit));
    assert_eq!(super::super::take_action(), None);
}

#[test]
fn restoring_all_attempts_every_live_instance_even_when_one_fails() {
    let first = NativeWindow::new();
    let failed = NativeWindow::new();
    let last = NativeWindow::new();
    let closed = NativeWindow::new();
    let windows =
        [&first, &failed, &closed, &last].map(|window| WindowRef::capture(window.0).unwrap());
    drop(closed);
    let mut restored = Vec::new();
    assert!(!restore_windows(&windows, |window| {
        restored.push(window.hwnd);
        window.hwnd != failed.0
    }));
    assert_eq!(restored, [first.0, failed.0, last.0]);
    assert!(windows[0].is_current() && windows[1].is_current() && windows[3].is_current());
    assert!(restore_windows(&[], |_| panic!("no instances to restore")));
}

#[test]
fn an_instance_closing_during_restore_does_not_report_a_spurious_failure() {
    let mut native = Some(NativeWindow::new());
    let window = WindowRef::capture(native.as_ref().unwrap().0).unwrap();
    assert!(restore_windows(&[window], |_| {
        drop(native.take());
        false
    }));
}

#[test]
fn restoring_all_reuses_hidden_minimized_and_visible_windows_and_only_queues_open_actions() {
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindowVisible, ShowWindow, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE,
    };
    extern "system" fn receive(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        _: usize,
    ) -> LRESULT {
        if msg == super::super::super::app_instances::RESTORE_MESSAGE {
            super::super::request_open(hwnd);
            return LRESULT(1);
        }
        unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
    }
    while super::super::take_action().is_some() {}
    let hidden = NativeWindow::new();
    let minimized = NativeWindow::new();
    let visible = NativeWindow::new();
    for window in [&hidden, &minimized, &visible] {
        assert!(unsafe { SetWindowSubclass(window.0, Some(receive), 814, 0) }.as_bool());
    }
    unsafe {
        let _ = ShowWindow(minimized.0, SW_SHOWMINNOACTIVE);
        let _ = ShowWindow(visible.0, SW_SHOWNOACTIVATE);
    }
    assert!(!unsafe { IsWindowVisible(hidden.0) }.as_bool());
    assert!(unsafe { IsIconic(minimized.0) }.as_bool());
    let windows =
        [&hidden, &minimized, &visible].map(|window| WindowRef::capture(window.0).unwrap());
    assert!(restore_windows(&windows, |window| {
        super::super::super::app_instances::restore_main_window(window.hwnd)
    }));
    for window in windows {
        assert!(window.is_current());
        assert!(unsafe { IsWindowVisible(window.hwnd) }.as_bool());
        assert!(!unsafe { IsIconic(window.hwnd) }.as_bool());
        assert_eq!(super::super::take_action(), Some(TrayAction::Open));
    }
    assert!(super::super::take_action().is_none());
}
