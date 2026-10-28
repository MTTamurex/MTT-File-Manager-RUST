//! Capture every live UI session before persisting and acknowledging global exit.
use super::{coordinator, model::Packet, protocol, window_ref::WindowRef, TrayAction};
use crate::{infrastructure::app_state_db::AppStateDb, tabs::TabManagerSession};
use crossbeam_channel::{bounded, Sender};
use eframe::egui::Context;
use std::{cell::RefCell, collections::HashMap, sync::Arc};
use windows::Win32::Foundation::HWND;

mod worker;

struct Pending {
    token: String,
    expected: HashMap<usize, WindowRef>,
    replies: Sender<(usize, TabManagerSession)>,
}

#[derive(Default)]
struct State {
    db: Option<Arc<AppStateDb>>,
    ctx: Option<Context>,
    busy: bool,
    pending: Option<Pending>,
    prepared: Option<(WindowRef, String)>,
    group_exit: bool,
    ready: bool,
    close_order: Option<crate::app::instance_sessions::ExitOrder>,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

pub(crate) fn bind(db: Arc<AppStateDb>, ctx: Context) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.db = Some(db);
        state.ctx = Some(ctx);
    });
}

pub(crate) fn initialize(db: Arc<AppStateDb>, ctx: Context) {
    STATE.with(|state| {
        *state.borrow_mut() = State {
            db: Some(db),
            ctx: Some(ctx),
            ..State::default()
        }
    });
}

pub(crate) fn remember_individual_close() {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state
            .close_order
            .get_or_insert_with(crate::app::instance_sessions::exit_order);
    });
}

pub(crate) fn individual_close_order() -> crate::app::instance_sessions::ExitOrder {
    STATE
        .with(|state| state.borrow().close_order)
        .unwrap_or_else(crate::app::instance_sessions::exit_order)
}

pub(crate) fn mark_ready() {
    STATE.with(|state| state.borrow_mut().ready = true);
}

pub(super) fn context() -> Option<Context> {
    STATE.with(|state| state.borrow().ctx.clone())
}
pub(crate) fn is_group_exit() -> bool {
    STATE.with(|state| state.borrow().group_exit)
}

fn enqueue(action: TrayAction) -> bool {
    let Some(ctx) = context() else {
        return false;
    };
    if !super::queue_action(action) {
        return false;
    }
    ctx.request_repaint();
    true
}

pub(super) fn request_all() -> bool {
    // A restore may still be launching authenticated child processes. Do not
    // accept a partial global-exit set and leave a new child outside it.
    if !STATE.with(|state| state.borrow().ready) || crate::app::instance_sessions::restoring() {
        return false;
    }
    if STATE.with(|state| state.borrow().busy) {
        return true;
    }
    if !enqueue(TrayAction::ExitAll) {
        return false;
    }
    STATE.with(|state| state.borrow_mut().busy = true);
    true
}

pub(crate) fn begin(owner: HWND, local: TabManagerSession) -> bool {
    let result = begin_inner(owner, local);
    if !result {
        STATE.with(|state| state.borrow_mut().busy = false);
    }
    result
}

fn begin_inner(owner: HWND, local: TabManagerSession) -> bool {
    let windows = super::super::app_instances::main_windows()
        .into_iter()
        .filter_map(WindowRef::capture)
        .collect();
    begin_with_windows(owner, local, windows)
}

fn begin_with_windows(owner: HWND, local: TabManagerSession, windows: Vec<WindowRef>) -> bool {
    let Some(owner_ref) = WindowRef::capture(owner) else {
        return false;
    };
    let Some(db) = STATE.with(|state| state.borrow().db.clone()) else {
        return false;
    };
    let Ok(id) = (unsafe { windows::Win32::System::Com::CoCreateGuid() }) else {
        return false;
    };
    let token = format!("{:032x}", id.to_u128());
    if !windows.contains(&owner_ref) || windows.len() > 128 {
        return false;
    }
    let identities: Vec<_> = windows
        .iter()
        .map(|window| (window.hwnd.0 as usize, window.process_id()))
        .collect();
    let expected = windows
        .into_iter()
        .filter(|window| *window != owner_ref)
        .map(|window| (window.hwnd.0 as usize, window))
        .collect();
    let (tx, rx) = bounded(128);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.busy = true;
        state.prepared = Some((owner_ref, token.clone()));
        state.pending = Some(Pending {
            token: token.clone(),
            expected,
            replies: tx,
        });
    });
    let owner_id = owner.0 as usize;
    let worker_token = token.clone();
    let started = std::thread::Builder::new()
        .name("session-exit".into())
        .spawn(move || {
            worker::run(owner_id, identities, local, worker_token, rx, db);
        })
        .is_ok();
    if !started {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.pending = None;
            state.prepared = None;
        });
    }
    started
}

pub(super) fn request_capture(sender: HWND, token: String) -> bool {
    if !STATE.with(|state| state.borrow().ready) || crate::app::instance_sessions::restoring() {
        return false;
    }
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let Some(window) = WindowRef::capture(sender) else {
        return false;
    };
    enqueue(TrayAction::CaptureSession {
        requester: sender.0 as usize,
        process_id: window.process_id(),
        token,
    })
}

pub(crate) fn reply(
    requester: usize,
    process_id: u32,
    token: String,
    session: TabManagerSession,
) -> bool {
    let requester = HWND(requester as *mut _);
    let Some(window) = WindowRef::capture(requester) else {
        return false;
    };
    if window.process_id() != process_id {
        return false;
    }
    let Some(local) = coordinator::local_window() else {
        return false;
    };
    STATE.with(|state| state.borrow_mut().prepared = Some((window, token.clone())));
    protocol::send(local, requester, &Packet::SessionReply { token, session })
}

pub(super) fn receive_reply(sender: HWND, token: &str, session: TabManagerSession) -> bool {
    let valid = session
        .to_json()
        .ok()
        .and_then(|json| TabManagerSession::from_json(&json));
    let Some(session) = valid else {
        return false;
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let Some(pending) = state
            .pending
            .as_mut()
            .filter(|pending| pending.token == token)
        else {
            return false;
        };
        let key = sender.0 as usize;
        if !pending
            .expected
            .get(&key)
            .is_some_and(|window| window.is_current())
        {
            return false;
        }
        if pending.replies.try_send((key, session)).is_err() {
            return false;
        }
        pending.expected.remove(&key);
        true
    })
}

pub(super) fn close(sender: HWND, token: &str) -> bool {
    let prepared = STATE.with(|state| {
        state
            .borrow()
            .prepared
            .as_ref()
            .is_some_and(|(window, expected)| {
                window.hwnd == sender && window.is_current() && expected == token
            })
    });
    let Some(local) = coordinator::local_window() else {
        return false;
    };
    if !prepared || !coordinator::request_exit(local) {
        return false;
    }
    // WM_CLOSE is posted to this same UI thread, so it cannot run before this
    // flag is installed. Individual teardown must not overwrite the saved group.
    STATE.with(|state| state.borrow_mut().group_exit = true);
    true
}

pub(super) fn finish(token: &str) -> bool {
    let matched = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state
            .pending
            .as_ref()
            .is_none_or(|pending| pending.token != token)
        {
            return false;
        }
        state.pending = None;
        state.prepared = None;
        state.busy = false;
        true
    });
    if matched {
        enqueue(TrayAction::ExitFailed);
    }
    matched
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
