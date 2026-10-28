use super::super::app_instances;
use super::model::{Packet, Snapshot};
use super::ownership::Ownership;
use super::window_ref::WindowRef;
use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::w;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    GetPropW, KillTimer, PostMessageW, RemovePropW, SetPropW, SetTimer, WM_CLOSE,
};

pub(crate) const TIMER_ID: usize = 0x4D545438;
pub(crate) const EXIT_MESSAGE: u32 = 0x8000 + 0x38;

#[derive(Default)]
struct State {
    hwnd: Option<HWND>,
    mutex: Option<Ownership>,
    owner: bool,
    timer_ready: bool,
    exiting: bool,
    local: Snapshot,
    peers: HashMap<usize, (WindowRef, Snapshot)>,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

#[cfg(test)]
pub(super) fn test_snapshot() -> Snapshot {
    STATE.with(|s| s.borrow().local.clone())
}

pub(crate) fn initialize(hwnd: HWND) {
    if initialize_with(
        hwnd,
        Ownership::new(w!("Local\\MTT.FileManager.Tray")),
        || unsafe { SetTimer(Some(hwnd), TIMER_ID, 1000, None) != 0 },
    ) {
        tick(hwnd);
    }
}

fn initialize_with(
    hwnd: HWND,
    ownership: windows::core::Result<Ownership>,
    start_timer: impl FnOnce() -> bool,
) -> bool {
    let mutex = match ownership {
        Ok(ownership) => Some(ownership),
        Err(error) => {
            log::warn!("[TRAY] Could not initialize shared icon ownership: {error}");
            None
        }
    };
    let timer_ready = mutex.is_some() && start_timer();
    if mutex.is_some() && !timer_ready {
        log::warn!("[TRAY] Could not start instance coordination timer");
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.hwnd = Some(hwnd);
        state.timer_ready = timer_ready;
        // Never participate in ownership without a working recovery timer.
        state.mutex = if timer_ready { mutex } else { None };
    });
    timer_ready
}

fn ready(hwnd: HWND) -> bool {
    STATE.with(|s| {
        let state = s.borrow();
        state.hwnd == Some(hwnd) && state.timer_ready && state.mutex.is_some() && !state.exiting
    })
}

pub(crate) fn local_window() -> Option<HWND> {
    STATE.with(|s| s.borrow().hwnd)
}

pub(crate) fn publish(mut snapshot: Snapshot) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        snapshot.hidden_at = state.local.hidden_at.clone();
        state.local = snapshot;
    });
}

pub(super) fn mark_hidden() {
    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    STATE.with(|s| {
        s.borrow_mut().local.hidden_at = Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
        ))
    });
}

pub(super) fn mark_visible() {
    STATE.with(|s| s.borrow_mut().local.hidden_at = None);
}

pub(super) fn receive_snapshot(hwnd: HWND, snapshot: Snapshot) {
    let Some(window) = WindowRef::capture(hwnd) else {
        return;
    };
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if state.owner {
            state.peers.insert(hwnd.0 as usize, (window, snapshot));
        }
    });
}

pub(crate) fn tick(hwnd: HWND) {
    if !ready(hwnd) || unsafe { !GetPropW(hwnd, w!("MTT.FileManager.Exiting")).is_invalid() } {
        return;
    }
    if unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(hwnd).as_bool() } {
        mark_visible();
    }
    let owner = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if !state.owner {
            state.owner = state.mutex.as_mut().is_some_and(Ownership::acquire);
        }
        state.owner
    });
    if owner {
        unsafe {
            if super::update_tray_icon(hwnd, window_count()) {
                let _ = SetPropW(
                    hwnd,
                    w!("MTT.FileManager.TrayOwner"),
                    Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
                );
            } else {
                let _ = RemovePropW(hwnd, w!("MTT.FileManager.TrayOwner"));
            }
        }
        STATE.with(|s| {
            s.borrow_mut()
                .peers
                .retain(|_, (window, _)| window.is_current())
        });
    } else if let Some(owner) = app_instances::main_windows()
        .into_iter()
        .find(|w| unsafe { !GetPropW(*w, w!("MTT.FileManager.TrayOwner")).is_invalid() })
    {
        let snapshot = STATE.with(|s| s.borrow().local.clone());
        super::protocol::send(hwnd, owner, &Packet::Snapshot(snapshot));
    }
}

pub(super) fn owns_icon(hwnd: HWND) -> bool {
    ready(hwnd) && STATE.with(|s| s.borrow().owner)
}

pub(super) fn window_count() -> usize {
    count_windows(&app_instances::main_windows())
}

fn count_windows(windows: &[HWND]) -> usize {
    windows
        .iter()
        .filter(|&&hwnd| WindowRef::capture(hwnd).is_some())
        .count()
}

pub(crate) fn available(hwnd: HWND) -> bool {
    if !ready(hwnd) {
        return false;
    }
    tick(hwnd);
    app_instances::main_windows()
        .into_iter()
        .any(|w| unsafe { !GetPropW(w, w!("MTT.FileManager.TrayOwner")).is_invalid() })
}

pub(super) fn windows() -> Vec<(WindowRef, Snapshot)> {
    let windows = app_instances::main_windows();
    STATE.with(|s| {
        let state = s.borrow();
        windows
            .into_iter()
            .filter_map(WindowRef::capture)
            .map(|w| {
                let snapshot = if Some(w.hwnd) == state.hwnd {
                    state.local.clone()
                } else {
                    state
                        .peers
                        .get(&(w.hwnd.0 as usize))
                        .filter(|(cached, _)| *cached == w)
                        .map(|(_, snapshot)| snapshot.clone())
                        .unwrap_or_default()
                };
                (w, snapshot)
            })
            .collect()
    })
}

pub(crate) fn request_exit(hwnd: HWND) -> bool {
    request_exit_with(hwnd, || unsafe {
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
    })
}

fn request_exit_with(hwnd: HWND, post_close: impl FnOnce() -> bool) -> bool {
    if STATE.with(|s| s.borrow().exiting) {
        return true;
    }
    // Post to our own UI thread first; it cannot dispatch WM_CLOSE until this
    // callback returns. Failed posting must not hide a still-running instance.
    if !post_close() {
        return false;
    }
    STATE.with(|s| s.borrow_mut().exiting = true);
    unsafe {
        let _ = SetPropW(
            hwnd,
            w!("MTT.FileManager.Exiting"),
            Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
        );
    }
    super::queue_action(super::TrayAction::Exit);
    true
}

pub(crate) fn shutdown(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_ID);
        let _ = RemovePropW(hwnd, w!("MTT.FileManager.TrayOwner"));
        let _ = SetPropW(
            hwnd,
            w!("MTT.FileManager.Exiting"),
            Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
        );
    }
    super::remove_tray_icon(hwnd);
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.exiting = true;
        state.timer_ready = false;
        state.owner = false;
        state.mutex = None;
    });
}

#[cfg(test)]
mod tests;
