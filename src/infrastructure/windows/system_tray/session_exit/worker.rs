use super::super::{commands, model::Packet, protocol, window_ref::WindowRef};
use crate::{infrastructure::app_state_db::AppStateDb, tabs::TabManagerSession};
use crossbeam_channel::Receiver;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use windows::Win32::Foundation::HWND;

pub(super) fn run(
    owner: usize,
    identities: Vec<(usize, u32)>,
    local: TabManagerSession,
    token: String,
    replies: Receiver<(usize, TabManagerSession)>,
    db: Arc<AppStateDb>,
) {
    let from = HWND(owner as *mut _);
    if !capture_and_close(from, &identities, local, &token, &replies, &db) {
        log::warn!("[SESSION] Global exit could not save or close every instance");
        protocol::send(from, from, &Packet::SessionExitFailed { token });
    }
}

fn capture_and_close(
    from: HWND,
    identities: &[(usize, u32)],
    local: TabManagerSession,
    token: &str,
    replies: &Receiver<(usize, TabManagerSession)>,
    db: &AppStateDb,
) -> bool {
    let Some(owner) = WindowRef::capture(from) else {
        return false;
    };
    let Some(windows) = identities
        .iter()
        .map(|&(hwnd, pid)| {
            WindowRef::capture(HWND(hwnd as *mut _)).filter(|window| window.process_id() == pid)
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    for window in &windows {
        if *window != owner
            && !protocol::send(
                from,
                window.hwnd,
                &Packet::RequestSession {
                    token: token.into(),
                },
            )
        {
            return false;
        }
    }
    let mut sessions = HashMap::from([(from.0 as usize, local)]);
    while sessions.len() < windows.len() {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        let Ok((id, session)) = replies.recv_timeout(remaining) else {
            return false;
        };
        sessions.insert(id, session);
    }
    if windows.iter().any(|window| !window.is_current()) {
        return false;
    }
    // Restore the tray owner first; the rest keep their own tabs and panels.
    let mut ordered = vec![sessions.remove(&(from.0 as usize)).unwrap()];
    for window in &windows {
        if *window != owner {
            let Some(session) = sessions.remove(&(window.hwnd.0 as usize)) else {
                return false;
            };
            ordered.push(session);
        }
    }
    if !crate::app::instance_sessions::save(db, ordered) {
        return false;
    }
    commands::exit_windows(owner, &windows, |window| {
        protocol::send(
            from,
            window.hwnd,
            &Packet::ExitSessionGroup {
                token: token.into(),
            },
        )
    })
}
