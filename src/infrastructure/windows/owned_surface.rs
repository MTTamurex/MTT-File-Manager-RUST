//! Owned top-level windows that must behave like children of the main window.
//!
//! The docked video is hosted in an owned popup (not a `WS_CHILD`) so DWM
//! composes it on its own instead of re-composing the whole main window at the
//! video's frame rate. Windows already keeps owned popups above their owner and
//! hides them on minimize; this registry covers the rest:
//! - follow the owner's client area when it moves, synchronously from `WM_MOVE`
//!   (egui may not paint during the modal move loop);
//! - hide with the owner when it is hidden (e.g. to the tray) and come back
//!   only if the UI still wants the surface visible.

use std::sync::Mutex;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindow, SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNOACTIVATE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Follower {
    hwnd: isize,
    /// Position inside the owner's client area, in physical pixels.
    offset: (i32, i32),
    wants_visible: bool,
}

static FOLLOWERS: Mutex<Vec<Follower>> = Mutex::new(Vec::new());
static OWNER_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn followers() -> std::sync::MutexGuard<'static, Vec<Follower>> {
    FOLLOWERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_follower(hwnd: HWND, update: impl FnOnce(&mut Follower)) {
    let mut list = followers();
    let key = hwnd.0 as isize;
    match list.iter_mut().find(|f| f.hwnd == key) {
        Some(follower) => update(follower),
        None => {
            let mut follower = Follower {
                hwnd: key,
                offset: (0, 0),
                wants_visible: false,
            };
            update(&mut follower);
            list.push(follower);
        }
    }
}

/// Record where `hwnd` sits inside the owner's client area.
pub fn set_offset(hwnd: HWND, offset: (i32, i32)) {
    with_follower(hwnd, |f| f.offset = offset);
}

/// Record whether the UI wants `hwnd` shown. Returns false while the owner is
/// hidden: the caller must not show the surface then.
pub fn set_wants_visible(hwnd: HWND, visible: bool) -> bool {
    with_follower(hwnd, |f| f.wants_visible = visible);
    !OWNER_HIDDEN.load(std::sync::atomic::Ordering::Acquire)
}

pub fn untrack(hwnd: HWND) {
    let key = hwnd.0 as isize;
    followers().retain(|f| f.hwnd != key);
}

/// Owner `WM_MOVE`: `client_origin` is its client area's top-left in screen pixels.
pub fn on_owner_moved(client_origin: (i32, i32)) {
    let snapshot = followers().clone();
    for follower in snapshot {
        let hwnd = HWND(follower.hwnd as _);
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            continue;
        }
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                None,
                client_origin.0 + follower.offset.0,
                client_origin.1 + follower.offset.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }
}

/// Owner `WM_SHOWWINDOW`.
pub fn on_owner_visibility(visible: bool) {
    OWNER_HIDDEN.store(!visible, std::sync::atomic::Ordering::Release);
    let snapshot = followers().clone();
    for follower in snapshot.into_iter().filter(|f| f.wants_visible) {
        let hwnd = HWND(follower.hwnd as _);
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            let _ = unsafe { ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        }
    }
}

/// Signed client-origin coordinates packed in a `WM_MOVE` lParam.
pub fn client_origin_from_move_lparam(lparam: isize) -> (i32, i32) {
    let x = (lparam & 0xffff) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_lparam_keeps_negative_coordinates() {
        // Client area at (-1910, 25) on a monitor left of the primary one.
        let lparam = ((25_i32 as u16 as isize) << 16) | (-1910_i32 as u16 as isize);
        assert_eq!(client_origin_from_move_lparam(lparam), (-1910, 25));
        assert_eq!(client_origin_from_move_lparam(0x0010_0020), (0x20, 0x10));
    }

    #[test]
    fn registry_tracks_offset_and_visibility_per_window() {
        let hwnd = HWND(0x5151 as _);
        set_offset(hwnd, (10, 20));
        assert!(set_wants_visible(hwnd, true));
        let entry = *followers().iter().find(|f| f.hwnd == 0x5151).unwrap();
        assert_eq!(
            entry,
            Follower {
                hwnd: 0x5151,
                offset: (10, 20),
                wants_visible: true
            }
        );
        untrack(hwnd);
        assert!(!followers().iter().any(|f| f.hwnd == 0x5151));
    }
}
