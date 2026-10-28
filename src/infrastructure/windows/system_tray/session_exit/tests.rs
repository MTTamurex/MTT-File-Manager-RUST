use super::super::{take_action, tests::NativeWindow};
use super::*;
use crate::tabs::TabManager;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

pub(super) fn reset() {
    while take_action().is_some() {}
    STATE.with(|state| *state.borrow_mut() = State::default());
}

#[test]
fn global_exit_is_queued_once_and_does_not_close_before_capture() {
    reset();
    bind(
        Arc::new(AppStateDb::new_in_memory().unwrap()),
        Context::default(),
    );
    mark_ready();
    assert!(request_all());
    assert!(request_all());
    assert_eq!(take_action(), Some(TrayAction::ExitAll));
    assert_eq!(take_action(), None);
    assert!(!is_group_exit());
}

#[test]
fn capture_requests_wake_the_ui_without_restoring_a_hidden_window() {
    reset();
    let peer = NativeWindow::new();
    bind(
        Arc::new(AppStateDb::new_in_memory().unwrap()),
        Context::default(),
    );
    mark_ready();
    assert!(!request_capture(peer.0, "invalid".into()));
    assert!(request_capture(peer.0, TOKEN.into()));
    assert_eq!(
        take_action(),
        Some(TrayAction::CaptureSession {
            requester: peer.0 .0 as usize,
            process_id: WindowRef::capture(peer.0).unwrap().process_id(),
            token: TOKEN.into()
        })
    );
    assert!(!unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(peer.0) }.as_bool());
    assert!(!is_group_exit());
}

#[test]
fn only_expected_replies_from_the_matching_transaction_are_accepted_once() {
    reset();
    let peer = NativeWindow::new();
    let outsider = NativeWindow::new();
    let (tx, rx) = bounded(4);
    STATE.with(|state| {
        state.borrow_mut().pending = Some(Pending {
            token: TOKEN.into(),
            expected: HashMap::from([(peer.0 .0 as usize, WindowRef::capture(peer.0).unwrap())]),
            replies: tx,
        })
    });
    let session = TabManagerSession::from_tab_manager(&TabManager::new());
    assert!(!receive_reply(peer.0, "stale", session.clone()));
    assert!(!receive_reply(outsider.0, TOKEN, session.clone()));
    assert!(receive_reply(peer.0, TOKEN, session.clone()));
    assert!(!receive_reply(peer.0, TOKEN, session.clone()));
    assert_eq!(rx.try_recv().unwrap().1, session);
    assert!(rx.try_recv().is_err());
}

#[test]
fn an_unprepared_group_close_cannot_disable_individual_session_saving() {
    reset();
    let peer = NativeWindow::new();
    assert!(!close(peer.0, TOKEN));
    assert!(!is_group_exit());
}

#[test]
fn the_first_accepted_close_keeps_its_order_until_final_persistence() {
    reset();
    remember_individual_close();
    let order = individual_close_order();
    let later = crate::app::instance_sessions::exit_order();
    assert!(later > order);
    remember_individual_close();
    assert_eq!(individual_close_order(), order);
}

#[test]
fn early_context_binding_does_not_enable_exit_before_startup_is_ready() {
    reset();
    let peer = NativeWindow::new();
    bind(
        Arc::new(AppStateDb::new_in_memory().unwrap()),
        Context::default(),
    );
    assert!(context().is_some());
    assert!(!request_all());
    assert!(!request_capture(peer.0, TOKEN.into()));
    assert!(take_action().is_none());
}

#[test]
fn failed_close_post_does_not_enable_group_exit_suppression() {
    reset();
    let sender = NativeWindow::new();
    let target = NativeWindow::new();
    let hwnd = target.0;
    drop(target);
    coordinator::test_set_local(hwnd);
    STATE.with(|state| {
        state.borrow_mut().prepared = Some((WindowRef::capture(sender.0).unwrap(), TOKEN.into()))
    });
    assert!(!close(sender.0, TOKEN));
    assert!(!is_group_exit());
}

#[test]
fn a_recycled_requester_identity_cannot_receive_a_private_session() {
    reset();
    let requester = NativeWindow::new();
    let wrong_pid = WindowRef::capture(requester.0)
        .unwrap()
        .process_id()
        .wrapping_add(1);
    let session = TabManagerSession::from_tab_manager(&TabManager::new());
    assert!(!reply(
        requester.0 .0 as usize,
        wrong_pid,
        TOKEN.into(),
        session
    ));
    assert!(STATE.with(|state| state.borrow().prepared.is_none()));
}

#[test]
fn failed_transactions_clear_busy_state_but_do_not_mark_an_instance_as_group_exiting() {
    reset();
    bind(
        Arc::new(AppStateDb::new_in_memory().unwrap()),
        Context::default(),
    );
    let (tx, _) = bounded(1);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.busy = true;
        state.pending = Some(Pending {
            token: TOKEN.into(),
            expected: HashMap::new(),
            replies: tx,
        });
    });
    assert!(!finish("stale"));
    assert!(finish(TOKEN));
    assert!(!STATE.with(|state| state.borrow().busy));
    assert!(!is_group_exit());
    assert_eq!(take_action(), Some(TrayAction::ExitFailed));
}
