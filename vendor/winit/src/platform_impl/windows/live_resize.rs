//! Opt-in DXGI live-resize ordering. Keep disabled for normal winit users.

use std::cell::Cell;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

use super::window_state::{WindowFlags, WindowState};
use crate::dpi::{PhysicalPosition, PhysicalSize};

thread_local! {
    static COMPOSITION_CREATION: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn composition_creation_enabled() -> bool {
    COMPOSITION_CREATION.with(Cell::get)
}

pub(crate) fn with_composition_creation<T>(create: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            COMPOSITION_CREATION.with(|enabled| enabled.set(self.0));
        }
    }
    let _restore = Restore(COMPOSITION_CREATION.with(|enabled| enabled.replace(true)));
    create()
}

#[derive(Default)]
pub(crate) struct LiveResize {
    enabled: bool,
    pending: Option<RECT>,
    interrupted: bool,
    prepared: Option<PhysicalSize<u32>>,
}

impl LiveResize {
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.prepared = None;
        }
    }

    pub fn pending_size(&self) -> Option<PhysicalSize<u32>> {
        self.pending.and_then(rect_size)
    }

    pub fn pending_position(&self) -> Option<PhysicalPosition<i32>> {
        self.pending
            .map(|rect| PhysicalPosition::new(rect.left, rect.top))
    }

    fn begin(
        &mut self,
        proposed: RECT,
        current: PhysicalSize<u32>,
        in_size_move: bool,
        can_dispatch: bool,
    ) -> Option<PhysicalSize<u32>> {
        if !self.enabled || self.pending.is_some() || !in_size_move || !can_dispatch {
            return None;
        }
        let size = rect_size(proposed)?;
        if size == current {
            return None;
        }
        self.pending = Some(proposed);
        self.interrupted = false;
        Some(size)
    }

    fn finish(&mut self, size: PhysicalSize<u32>) -> bool {
        let ready = self.enabled && !self.interrupted;
        self.prepared = ready.then_some(size);
        ready
    }

    pub fn on_native_size(&mut self, size: PhysicalSize<u32>) -> bool {
        if self.pending.is_some() {
            // Native geometry changed while rendering. Do not suppress the
            // reconciliation event or claim that the early frame is current.
            self.interrupted = true;
            self.prepared = None;
            return false;
        }
        self.prepared.take() == Some(size)
    }
}

fn rect_size(rect: RECT) -> Option<PhysicalSize<u32>> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    (width > 0 && height > 0).then(|| PhysicalSize::new(width as u32, height as u32))
}

struct PendingResize<'a>(&'a Mutex<WindowState>);

impl Drop for PendingResize<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap().live_resize.pending = None;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Preparation {
    Skipped,
    Presented,
    Interrupted,
}

pub(super) fn present_before_resize(
    hwnd: HWND,
    state: &Mutex<WindowState>,
    proposed: RECT,
    can_dispatch: bool,
    present: impl FnOnce(PhysicalSize<u32>),
) -> Preparation {
    let size = {
        let mut state = state.lock().unwrap();
        if !state.live_resize.enabled || !can_dispatch {
            return Preparation::Skipped;
        }
        let mut current: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetClientRect(hwnd, &mut current) } == 0 {
            return Preparation::Skipped;
        }
        let Some(current) = rect_size(current) else {
            return Preparation::Skipped;
        };
        let in_size_move = state
            .window_flags
            .contains(WindowFlags::MARKER_IN_SIZE_MOVE);
        state
            .live_resize
            .begin(proposed, current, in_size_move, can_dispatch)
    };
    if let Some(size) = size {
        // Report the prospective geometry only for this synchronous callback.
        // The guard restores normal Win32 queries even if the renderer unwinds.
        let _pending = PendingResize(state);
        present(size);
        return if state.lock().unwrap().live_resize.finish(size) {
            Preparation::Presented
        } else {
            Preparation::Interrupted
        };
    }
    Preparation::Skipped
}

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
