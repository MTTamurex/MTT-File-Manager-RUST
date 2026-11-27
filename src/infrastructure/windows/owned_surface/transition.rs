//! Size transitions of the owner: its surfaces are hosted as children from the
//! first size change until the owner settles, then return to popups.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindowVisible, KillTimer, SetTimer, SWP_NOSIZE, WINDOWPOS,
};

use super::{follow, owned_by, reparent, with_follower};

/// Owner timer that ends a size transition once the owner settles.
pub const TRANSITION_TIMER_ID: usize = 0x4D54_5453;
/// Longer than the maximize/restore animation; restarted by every size change.
const SETTLE_MS: u32 = 400;
/// Where Windows parks a minimized window.
const MINIMIZED_POSITION: i32 = -32000;

/// Owner `WM_WINDOWPOSCHANGING`, before the change is applied. A new size
/// (live resize, maximize/restore) hosts its surfaces as children. Outside a
/// modal size/move loop the transition ends once the owner settles; inside
/// one, `WM_EXITSIZEMOVE` schedules the end. Minimizing needs nothing: the
/// surfaces hide with the owner.
pub fn on_owner_pos_changing(owner: HWND, window_pos: &WINDOWPOS, in_size_move: bool) {
    if !resizes(owner, window_pos) {
        return;
    }
    for hwnd in owned_by(owner, false) {
        if reparent::to_child(hwnd, owner) {
            with_follower(hwnd, |f| {
                f.child = true;
                f.applied_bounds = None;
            });
            follow(hwnd);
        }
    }
    if !in_size_move {
        schedule_end_size_transition(owner);
    }
}

/// (Re)starts the settle timer that returns the surfaces to popups.
pub fn schedule_end_size_transition(owner: HWND) {
    if !owned_by(owner, true).is_empty() {
        let _ = unsafe { SetTimer(Some(owner), TRANSITION_TIMER_ID, SETTLE_MS, None) };
    }
}

/// Settle timer. Keeps the children (and the timer) while the owner is still
/// in a size/move loop, minimized or hidden: a popup would show without it.
pub fn end_size_transition(owner: HWND, in_size_move: bool) {
    let settled = !in_size_move
        && !unsafe { IsIconic(owner) }.as_bool()
        && unsafe { IsWindowVisible(owner) }.as_bool();
    if !settled {
        return;
    }
    let _ = unsafe { KillTimer(Some(owner), TRANSITION_TIMER_ID) };
    for hwnd in owned_by(owner, true) {
        if reparent::to_popup(hwnd, owner) {
            with_follower(hwnd, |f| {
                f.child = false;
                f.applied_bounds = None;
            });
            follow(hwnd);
        }
    }
}

fn resizes(owner: HWND, window_pos: &WINDOWPOS) -> bool {
    if window_pos.flags.contains(SWP_NOSIZE)
        || (window_pos.x, window_pos.y) == (MINIMIZED_POSITION, MINIMIZED_POSITION)
    {
        return false;
    }
    let mut current = RECT::default();
    if unsafe { GetWindowRect(owner, &mut current) }.is_err() {
        return false;
    }
    (window_pos.cx, window_pos.cy) != (current.right - current.left, current.bottom - current.top)
}
