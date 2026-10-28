//! Exercise the real WM_COPYDATA handshake with hidden fixture windows only.
use super::super::{take_action, tests::NativeWindow};
use super::*;
use crate::tabs::TabManager;
use std::cell::Cell;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

const FRAME: u32 = WM_APP + 71;
const MUTATE: u32 = WM_APP + 72;
const STOP: u32 = WM_APP + 73;
const DEADLINE: usize = 7001;
thread_local! {
    static MODEL: RefCell<Option<TabManager>> = const { RefCell::new(None) };
    static CLOSED: Cell<bool> = const { Cell::new(false) };
    static FAILED: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_COPYDATA => {
            let accepted =
                unsafe { protocol::receive(HWND(wparam.0 as *mut _), lparam) }.unwrap_or(false);
            unsafe {
                let _ = PostMessageW(Some(hwnd), FRAME, WPARAM(0), LPARAM(0));
            }
            LRESULT(isize::from(accepted))
        }
        FRAME => {
            while let Some(action) = take_action() {
                match action {
                    TrayAction::CaptureSession {
                        requester,
                        process_id,
                        token,
                    } => {
                        let session = MODEL.with(|model| {
                            model
                                .borrow()
                                .as_ref()
                                .map(TabManagerSession::from_tab_manager)
                        });
                        if !session
                            .is_some_and(|session| reply(requester, process_id, token, session))
                        {
                            FAILED.set(true);
                        }
                    }
                    TrayAction::Exit => CLOSED.set(true),
                    TrayAction::ExitFailed => FAILED.set(true),
                    _ => {}
                }
            }
            LRESULT(0)
        }
        MUTATE => {
            MODEL.with(|model| {
                if let Some(model) = model.borrow_mut().as_mut() {
                    model.new_tab_at(r"D:\Other");
                }
            });
            LRESULT(1)
        }
        WM_CLOSE | STOP => {
            CLOSED.set(true);
            LRESULT(0)
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

fn fixture(db: Arc<AppStateDb>, path: &str) -> NativeWindow {
    super::tests::reset();
    CLOSED.set(false);
    FAILED.set(false);
    let window = NativeWindow::new();
    MODEL.with(|model| *model.borrow_mut() = Some(TabManager::new_at_path(path)));
    coordinator::test_set_local(window.0);
    bind(db, Context::default());
    mark_ready();
    assert!(unsafe { SetWindowSubclass(window.0, Some(procedure), 7001, 0) }.as_bool());
    window
}

fn pump(hwnd: HWND, done: impl Fn() -> bool) {
    assert_ne!(unsafe { SetTimer(Some(hwnd), DEADLINE, 5000, None) }, 0);
    let mut message = MSG::default();
    while !done() {
        assert!(unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0);
        assert!(
            !(message.message == WM_TIMER && message.wParam.0 == DEADLINE),
            "session handshake timed out"
        );
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    unsafe {
        let _ = KillTimer(Some(hwnd), DEADLINE);
    }
}

#[test]
fn global_exit_captures_fresh_hidden_peer_state_and_persists_before_any_close() {
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let owner = fixture(db.clone(), r"C:\Example");
    let (ready_tx, ready_rx) = bounded(1);
    let peer_db = db.clone();
    let peer = std::thread::spawn(move || {
        let window = fixture(peer_db.clone(), r"E:\Example");
        ready_tx.send(window.0 .0 as usize).unwrap();
        pump(window.0, || CLOSED.get() || FAILED.get());
        assert!(!FAILED.get());
        assert!(is_group_exit());
        assert!(peer_db
            .get_preference(crate::app::instance_sessions::KEY)
            .is_some());
        assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
    });
    let peer_hwnd = HWND(ready_rx.recv().unwrap() as *mut _);
    // Change the peer after it was registered. A cached periodic snapshot would
    // miss this tab; the prepare/reply handshake must capture its live state.
    unsafe {
        SendMessageW(peer_hwnd, MUTATE, None, None);
    }
    let local =
        MODEL.with(|model| TabManagerSession::from_tab_manager(model.borrow().as_ref().unwrap()));
    assert!(begin_with_windows(
        owner.0,
        local.clone(),
        vec![
            WindowRef::capture(owner.0).unwrap(),
            WindowRef::capture(peer_hwnd).unwrap()
        ]
    ));
    pump(owner.0, || CLOSED.get() || FAILED.get());
    assert!(!FAILED.get());
    assert!(is_group_exit());
    let record: serde_json::Value = serde_json::from_str(
        &db.get_preference(crate::app::instance_sessions::KEY)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["sessions"].as_array().unwrap().len(), 2);
    let saved_owner = TabManagerSession::from_json(&record["sessions"][0].to_string()).unwrap();
    let saved_peer = TabManagerSession::from_json(&record["sessions"][1].to_string()).unwrap();
    assert_eq!(saved_owner, local);
    assert_eq!(saved_peer.tabs.len(), 2);
    assert_eq!(saved_peer.tabs[1].path, r"D:\Other");
    assert_eq!(saved_peer.active_tab, 1);
    peer.join().unwrap();
}

#[test]
fn an_unresponsive_peer_keeps_all_instances_and_the_previous_restart_record() {
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let owner = fixture(db.clone(), r"C:\Example");
    let peer = NativeWindow::new(); // No protocol receiver: cannot acknowledge capture.
    let local =
        MODEL.with(|model| TabManagerSession::from_tab_manager(model.borrow().as_ref().unwrap()));
    assert!(crate::app::instance_sessions::save(
        &db,
        vec![local.clone()]
    ));
    let previous = db
        .get_preference(crate::app::instance_sessions::KEY)
        .unwrap();
    assert!(begin_with_windows(
        owner.0,
        local,
        vec![
            WindowRef::capture(owner.0).unwrap(),
            WindowRef::capture(peer.0).unwrap()
        ]
    ));
    pump(owner.0, || FAILED.get());
    assert!(!CLOSED.get());
    assert!(!is_group_exit());
    assert!(WindowRef::capture(owner.0).is_some() && WindowRef::capture(peer.0).is_some());
    assert_eq!(
        db.get_preference(crate::app::instance_sessions::KEY)
            .unwrap(),
        previous
    );
}

#[test]
fn a_failed_database_commit_does_not_close_or_suppress_the_owner() {
    let db = Arc::new(AppStateDb::new_in_memory().unwrap());
    let owner = fixture(db.clone(), r"C:\Example");
    let local =
        MODEL.with(|model| TabManagerSession::from_tab_manager(model.borrow().as_ref().unwrap()));
    assert!(crate::app::instance_sessions::save(
        &db,
        vec![local.clone()]
    ));
    let previous = db
        .get_preference(crate::app::instance_sessions::KEY)
        .unwrap();
    let guard = db.test_hold_writer();
    assert!(begin_with_windows(
        owner.0,
        local,
        vec![WindowRef::capture(owner.0).unwrap()]
    ));
    pump(owner.0, || FAILED.get());
    drop(guard);
    assert!(!CLOSED.get());
    assert!(!is_group_exit());
    assert!(WindowRef::capture(owner.0).is_some());
    assert_eq!(
        db.get_preference(crate::app::instance_sessions::KEY)
            .unwrap(),
        previous
    );
}
