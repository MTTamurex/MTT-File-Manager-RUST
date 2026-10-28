use super::super::super::{model::Snapshot, tests::NativeWindow};
use super::super::{
    input::{Hit, Part},
    tests::state,
    Model,
};
use super::*;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::SetPropW;

extern "system" fn accept_close(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == super::super::super::coordinator::EXIT_MESSAGE {
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

#[test]
fn closing_a_peer_removes_only_its_rows_and_keeps_the_popup_and_other_expansions() {
    let owner = NativeWindow::new();
    let peer = NativeWindow::new();
    let mut state = state(owner.0);
    let snapshot = state.model.windows[0].1.clone();
    let target = WindowRef::capture(peer.0).unwrap();
    state.model = Model::new(vec![
        (WindowRef::capture(owner.0).unwrap(), snapshot.clone()),
        (target, snapshot),
    ]);
    state.model.toggle(0);
    let index = state.model.rows.len() - 1;
    assert!(unsafe { SetWindowSubclass(peer.0, Some(accept_close), 811, 0) }.as_bool());
    state.click(Hit {
        index,
        part: Part::Close,
    });
    assert!(!state.done);
    let plan = take_plan(&mut state).unwrap();
    apply(&mut state, execute(plan));
    assert!(!state.done);
    assert_eq!(state.model.windows.len(), 1);
    assert_eq!(state.model.windows[0].0.hwnd, owner.0);
    assert_eq!(state.model.rows[0].expanded, Some(true));
    assert!(state.resize);
    assert!(state.command.is_none() && state.pressed.is_none() && state.hot_close.is_none());
}

#[test]
fn failed_close_keeps_the_entry_and_the_menu_open() {
    let owner = NativeWindow::new();
    let peer = NativeWindow::new();
    let mut state = state(owner.0);
    state.model = Model::new(
        [&owner, &peer]
            .map(|window| (WindowRef::capture(window.0).unwrap(), Snapshot::default()))
            .into(),
    );
    state.click(Hit {
        index: 1,
        part: Part::Close,
    });
    let plan = take_plan(&mut state).unwrap();
    apply(&mut state, execute(plan));
    assert!(!state.done);
    assert_eq!(state.model.windows.len(), 2);
    assert!(WindowRef::capture(peer.0).is_some());
}

#[test]
fn closing_the_host_requires_an_acknowledged_handoff_before_it_can_exit() {
    let owner = NativeWindow::new();
    let peer = NativeWindow::new();
    let mut state = state(owner.0);
    state.model = Model::new(
        [&owner, &peer]
            .map(|window| (WindowRef::capture(window.0).unwrap(), Snapshot::default()))
            .into(),
    );
    state.click(Hit {
        index: 0,
        part: Part::Close,
    });
    let plan = take_plan(&mut state).unwrap();
    assert!(plan.continuation.is_some());
    // This fixture has no continuation receiver. Never close the host and lose
    // access to its peers when another process cannot continue the menu.
    apply(&mut state, execute(plan));
    assert!(!state.done);
    assert!(WindowRef::capture(owner.0).is_some());
    assert_eq!(state.model.windows.len(), 2);
}

#[test]
fn the_popup_closes_when_the_last_instance_is_removed() {
    let owner = NativeWindow::new();
    let mut state = state(owner.0);
    let target = state.model.windows[0].0;
    apply(&mut state, Result::Closed(target));
    assert!(state.done);
    assert!(state.model.windows.is_empty());
}
