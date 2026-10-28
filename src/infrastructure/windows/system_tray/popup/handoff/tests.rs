use super::super::super::{model::Snapshot, tests::NativeWindow};
use super::super::tests::state;
use super::*;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, LRESULT};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SetPropW, WM_COPYDATA};

extern "system" fn receiver(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == WM_COPYDATA {
        return LRESULT(isize::from(
            unsafe { protocol::receive(HWND(wp.0 as *mut _), lp) }.unwrap_or(false),
        ));
    }
    if msg == coordinator::EXIT_MESSAGE {
        return LRESULT(isize::from(
            unsafe {
                SetPropW(
                    hwnd,
                    w!("MTT.FileManager.Exiting"),
                    Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
                )
            }
            .is_ok(),
        ));
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

fn reset() {
    PENDING.with(|pending| pending.borrow_mut().take());
    ACTIVE.with(|active| active.set(false));
}

#[test]
fn acknowledged_native_handoff_preserves_anchor_expansion_snapshots_and_surviving_host() {
    reset();
    let source = NativeWindow::new();
    let destination = NativeWindow::new();
    let mut state = state(source.0);
    let snapshot = state.model.windows[0].1.clone();
    state
        .model
        .windows
        .push((WindowRef::capture(destination.0).unwrap(), snapshot.clone()));
    state.model.rebuild();
    state.model.toggle(1);
    state.model.scroll = 30;
    for window in [&source, &destination] {
        assert!(unsafe { SetWindowSubclass(window.0, Some(receiver), 812, 0) }.as_bool());
    }
    coordinator::test_set_local(destination.0);
    assert!(send(source.0, capture(&state)));
    // Acknowledging the queued handoff alone must not close the old host.
    assert!(WindowRef::capture(source.0).is_some());
    let (model, anchor) = take_model(destination.0).unwrap();
    assert!(WindowRef::capture(source.0).is_none());
    assert!(WindowRef::capture(destination.0).is_some());
    assert_eq!((anchor.x, anchor.y), (state.anchor.x, state.anchor.y));
    assert_eq!(model.windows.len(), 1);
    assert_eq!(model.windows[0].1, snapshot);
    assert_eq!(model.rows[0].expanded, Some(true));
    assert_eq!(model.scroll, 30);
    assert!(!unsafe { IsWindowVisible(destination.0) }.as_bool());
    assert!(take_model(destination.0).is_none());
}

#[test]
fn invalid_duplicate_or_stale_identities_cannot_queue_a_handoff() {
    reset();
    let source = NativeWindow::new();
    let destination = NativeWindow::new();
    coordinator::test_set_local(destination.0);
    let mut state = state(source.0);
    state.model.windows.push((
        WindowRef::capture(destination.0).unwrap(),
        Snapshot::default(),
    ));
    let mut invalid = capture(&state);
    invalid.windows[1].process_id = invalid.windows[1].process_id.wrapping_add(1);
    assert!(!receive(source.0, invalid));
    let mut duplicate = capture(&state);
    duplicate.windows.push(PopupInstance {
        handle: source.0 .0 as usize,
        process_id: WindowRef::capture(source.0).unwrap().process_id(),
        snapshot: Snapshot::default(),
        expanded: false,
    });
    assert!(!receive(source.0, duplicate));
    ACTIVE.with(|active| active.set(true));
    assert!(!receive(source.0, capture(&state)));
    ACTIVE.with(|active| active.set(false));
    assert!(PENDING.with(|pending| pending.borrow().is_none()));
    assert!(WindowRef::capture(source.0).is_some());
}

#[test]
fn an_active_popup_can_continue_on_another_ui_thread_without_showing_its_main_window() {
    use std::{sync::mpsc, time::Duration};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, KillTimer, SetTimer, MSG, WM_APP, WM_TIMER,
    };
    reset();
    let source = NativeWindow::new();
    let source_id = source.0 .0 as usize;
    assert!(unsafe { SetWindowSubclass(source.0, Some(receiver), 813, 0) }.as_bool());
    let (ready_tx, ready_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    const DONE: u32 = WM_APP + 0x7b;
    const DEADLINE: usize = 813;
    let peer = std::thread::spawn(move || {
        reset();
        let destination = NativeWindow::new();
        coordinator::test_set_local(destination.0);
        assert!(unsafe { SetWindowSubclass(destination.0, Some(receiver), 813, 0) }.as_bool());
        ready_tx.send(destination.0 .0 as usize).unwrap();
        assert_ne!(
            unsafe { SetTimer(Some(destination.0), DEADLINE, 5000, None) },
            0
        );
        let mut msg = MSG::default();
        loop {
            assert!(unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0);
            assert!(
                !(msg.message == WM_TIMER && msg.wParam.0 == DEADLINE),
                "continuation timed out"
            );
            if msg.message == POPUP_CONTINUE_MESSAGE {
                break;
            }
            unsafe {
                DispatchMessageW(&msg);
            }
        }
        let (model, _) = take_model(destination.0).unwrap();
        result_tx
            .send((
                model.windows.len(),
                model.windows[0].0.hwnd == destination.0,
                model.rows[0].expanded,
                unsafe { IsWindowVisible(destination.0) }.as_bool(),
            ))
            .unwrap();
        unsafe {
            let _ = KillTimer(Some(destination.0), DEADLINE);
            PostMessageW(Some(HWND(source_id as *mut _)), DONE, WPARAM(0), LPARAM(0)).unwrap();
        }
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });
    let destination = HWND(ready_rx.recv_timeout(Duration::from_secs(5)).unwrap() as *mut _);
    let mut state = state(source.0);
    let snapshot = state.model.windows[0].1.clone();
    state
        .model
        .windows
        .push((WindowRef::capture(destination).unwrap(), snapshot));
    state.model.rebuild();
    state.model.toggle(1);
    ACTIVE.with(|active| active.set(true));
    assert!(send(source.0, capture(&state)));
    assert_ne!(unsafe { SetTimer(Some(source.0), DEADLINE, 5000, None) }, 0);
    let mut msg = MSG::default();
    loop {
        assert!(unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0);
        assert!(
            !(msg.message == WM_TIMER && msg.wParam.0 == DEADLINE),
            "host close timed out"
        );
        if msg.message == DONE {
            break;
        }
        unsafe {
            DispatchMessageW(&msg);
        }
    }
    unsafe {
        let _ = KillTimer(Some(source.0), DEADLINE);
    }
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        (1, true, Some(true), false)
    );
    assert!(WindowRef::capture(source.0).is_none());
    release_tx.send(()).unwrap();
    peer.join().unwrap();
    ACTIVE.with(|active| active.set(false));
}
