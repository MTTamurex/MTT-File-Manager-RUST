//! Owned top-level windows that must behave like children of the main window.
//!
//! The docked video is hosted in an owned popup (not a `WS_CHILD`) so DWM
//! composes it on its own instead of re-composing the whole main window at the
//! video's frame rate. Windows already keeps owned popups above their owner and
//! hides them on minimize; this registry covers the rest:
//! - follow the owner's client area synchronously from its `WM_MOVE`/`WM_SIZE`
//!   (egui lays out a frame later, or not at all inside the modal size/move
//!   loop). The last egui layout is projected onto the new client size through
//!   its [`Anchor`], so a panel docked to the right edge stays there while the
//!   window is resized, maximized or restored;
//! - clip to the owner's client area, which Windows does only for children;
//! - while the owner changes size (live resize, maximize/restore animation),
//!   host the surface as a real child so DWM composes it with the owner: a
//!   popup cannot follow the animation or the owner's lagging content, and
//!   showed up outside the window for a moment;
//! - hide with the owner when it is hidden (e.g. to the tray) and come back
//!   only if the UI still wants the surface visible.

mod reparent;
mod transition;

pub use transition::{
    end_size_transition, on_owner_pos_changing, schedule_end_size_transition, TRANSITION_TIMER_ID,
};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsIconic, IsWindow, SetWindowPos, ShowWindow, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE,
};

use super::surface_region::{self, Cutout};

/// Which client edges the surface keeps its distance to when the client area
/// changes size before the UI lays it out again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Fixed size, fixed distance to the left and top edges.
    TopLeft,
    /// Fixed size, fixed distance to the right and top edges.
    TopRight,
    /// Fixed distance to every edge: the size follows the client area.
    Fill,
}

/// Where the UI placed a surface, in physical pixels of the owner's client area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub position: (i32, i32),
    pub size: (i32, i32),
    /// Client size the layout was computed for.
    pub client_size: (i32, i32),
    pub anchor: Anchor,
}

impl Layout {
    /// Position and size of the surface in a client area of `client_size`.
    fn project(&self, client_size: (i32, i32)) -> ((i32, i32), (i32, i32)) {
        let grow_x = client_size.0 - self.client_size.0;
        let grow_y = client_size.1 - self.client_size.1;
        let ((x, y), (width, height)) = (self.position, self.size);
        match self.anchor {
            Anchor::TopLeft => ((x, y), (width, height)),
            Anchor::TopRight => ((x + grow_x, y), (width, height)),
            Anchor::Fill => ((x, y), ((width + grow_x).max(1), (height + grow_y).max(1))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppliedRegion {
    size: (i32, i32),
    clip: Option<(i32, i32, i32, i32)>,
    cutouts: Vec<Cutout>,
}

#[derive(Clone, Debug, Default)]
struct Follower {
    hwnd: isize,
    owner: isize,
    layout: Option<Layout>,
    cutouts: Vec<Cutout>,
    /// Hosted as a child of the owner during a size transition.
    child: bool,
    /// Bounds `(x, y, width, height)` last given to the window: screen
    /// coordinates as a popup, client coordinates as a child.
    applied_bounds: Option<(i32, i32, i32, i32)>,
    applied_region: Option<AppliedRegion>,
    wants_visible: bool,
}

static FOLLOWERS: Mutex<Vec<Follower>> = Mutex::new(Vec::new());
static OWNER_HIDDEN: AtomicBool = AtomicBool::new(false);

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
                ..Follower::default()
            };
            update(&mut follower);
            list.push(follower);
        }
    }
}

/// Places `hwnd` inside `owner`'s client area as laid out by the UI.
pub fn place(hwnd: HWND, owner: HWND, layout: Layout) {
    with_follower(hwnd, |f| {
        f.owner = owner.0 as isize;
        f.layout = Some(layout);
    });
    follow(hwnd);
}

/// Parts of `hwnd` (surface coordinates) where egui overlays show through.
pub fn set_cutouts(hwnd: HWND, cutouts: Vec<Cutout>) {
    with_follower(hwnd, |f| f.cutouts = cutouts);
    follow(hwnd);
}

/// Record whether the UI wants `hwnd` shown. Returns false while the owner is
/// hidden: the caller must not show the surface then.
pub fn set_wants_visible(hwnd: HWND, visible: bool) -> bool {
    with_follower(hwnd, |f| f.wants_visible = visible);
    !OWNER_HIDDEN.load(Ordering::Acquire)
}

pub fn untrack(hwnd: HWND) {
    let key = hwnd.0 as isize;
    followers().retain(|f| f.hwnd != key);
}

fn owned_by(owner: HWND, child: bool) -> Vec<HWND> {
    let key = owner.0 as isize;
    followers()
        .iter()
        .filter(|f| f.owner == key && f.child == child)
        .map(|f| HWND(f.hwnd as _))
        .collect()
}

/// Owner `WM_MOVE`/`WM_SIZE`: re-project every popup it owns. A child moves
/// with the owner and is only placed by the UI, in step with its content.
pub fn on_owner_geometry(owner: HWND) {
    for hwnd in owned_by(owner, false) {
        follow(hwnd);
    }
}

/// Owner `WM_SHOWWINDOW`.
pub fn on_owner_visibility(visible: bool) {
    OWNER_HIDDEN.store(!visible, Ordering::Release);
    let snapshot = followers().clone();
    for follower in snapshot.into_iter().filter(|f| f.wants_visible) {
        let hwnd = HWND(follower.hwnd as _);
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            let _ = unsafe { ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        }
    }
}

/// Owner client area: screen origin and size.
fn owner_client(owner: HWND) -> Option<((i32, i32), (i32, i32))> {
    if !unsafe { IsWindow(Some(owner)) }.as_bool() || unsafe { IsIconic(owner) }.as_bool() {
        return None;
    }
    let mut origin = POINT::default();
    let mut client = RECT::default();
    unsafe {
        ClientToScreen(owner, &mut origin).as_bool().then_some(())?;
        GetClientRect(owner, &mut client).ok()?;
    }
    Some(((origin.x, origin.y), (client.right, client.bottom)))
}

/// Moves and clips one surface for the owner's current client area. Native
/// calls run outside the registry lock: they send messages synchronously.
fn follow(hwnd: HWND) {
    let key = hwnd.0 as isize;
    let Some(follower) = followers().iter().find(|f| f.hwnd == key).cloned() else {
        return;
    };
    let Some(layout) = follower.layout else {
        return;
    };
    let owner = HWND(follower.owner as _);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return;
    }
    let Some((origin, client_size)) = owner_client(owner) else {
        return;
    };

    let (position, size) = layout.project(client_size);
    let origin = if follower.child { (0, 0) } else { origin };
    let bounds = (origin.0 + position.0, origin.1 + position.1, size.0, size.1);
    // After a host switch (no bounds yet), FRAMECHANGED applies the new style.
    let flags = if follower.applied_bounds.is_none() {
        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED
    } else {
        SWP_NOZORDER | SWP_NOACTIVATE
    };
    let moved = follower.applied_bounds != Some(bounds)
        && unsafe { SetWindowPos(hwnd, None, bounds.0, bounds.1, bounds.2, bounds.3, flags) }
            .is_ok();

    let region = AppliedRegion {
        size,
        clip: surface_region::clip_to_client(position, size, client_size),
        cutouts: follower.cutouts,
    };
    let reshaped = follower.applied_region.as_ref() != Some(&region)
        && surface_region::apply(hwnd, region.size, region.clip, &region.cutouts);

    if moved || reshaped {
        with_follower(hwnd, |f| {
            if moved {
                f.applied_bounds = Some(bounds);
            }
            if reshaped {
                f.applied_region = Some(region);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(anchor: Anchor) -> Layout {
        Layout {
            position: (500, 40),
            size: (280, 160),
            client_size: (800, 600),
            anchor,
        }
    }

    #[test]
    fn top_left_layout_ignores_client_size() {
        assert_eq!(
            layout(Anchor::TopLeft).project((1200, 900)),
            ((500, 40), (280, 160))
        );
    }

    #[test]
    fn top_right_layout_keeps_the_distance_to_the_right_edge() {
        // Right edge 20 px from the client's right edge before and after.
        assert_eq!(
            layout(Anchor::TopRight).project((1200, 900)),
            ((900, 40), (280, 160))
        );
        assert_eq!(
            layout(Anchor::TopRight).project((640, 300)),
            ((340, 40), (280, 160))
        );
    }

    #[test]
    fn fill_layout_follows_the_client_size() {
        assert_eq!(
            layout(Anchor::Fill).project((1200, 900)),
            ((500, 40), (680, 460))
        );
        assert_eq!(
            layout(Anchor::Fill).project((100, 100)),
            ((500, 40), (1, 1))
        );
    }

    #[test]
    fn registry_tracks_visibility_per_window() {
        let hwnd = HWND(0x5151 as _);
        assert!(set_wants_visible(hwnd, true));
        assert!(followers()
            .iter()
            .any(|f| f.hwnd == 0x5151 && f.wants_visible));
        untrack(hwnd);
        assert!(!followers().iter().any(|f| f.hwnd == 0x5151));
    }
}
