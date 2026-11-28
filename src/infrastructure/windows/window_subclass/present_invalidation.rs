//! Tells the renderer when the main window needs a freshly presented frame.
//!
//! The vendored `egui-wgpu` skips rendering and presenting a frame identical to
//! the one on screen, so an idle window costs no GPU time. DWM does not keep
//! that image across every window-manager transition (minimize discards it),
//! so the skip is enabled only while this subclass reports those transitions.
//!
//! The skip also stays off when the render GPU does not drive the monitor.
//! Each presented frame is then copied between GPUs, and the first present
//! after a run of skipped frames stalls the UI thread for about 500 ms.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::GetUpdateRect;
use windows::Win32::UI::WindowsAndMessaging::{
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_PAINT, WM_SHOWWINDOW, WM_SIZE,
};

static SUBCLASS_ACTIVE: AtomicBool = AtomicBool::new(false);
static CROSS_ADAPTER: AtomicBool = AtomicBool::new(false);

fn apply() {
    eframe::egui_wgpu::winit::set_identical_frame_skip(
        SUBCLASS_ACTIVE.load(Ordering::Acquire) && !CROSS_ADAPTER.load(Ordering::Acquire),
    );
}

pub(super) fn enable() {
    SUBCLASS_ACTIVE.store(true, Ordering::Release);
    apply();
}

pub(super) fn disable() {
    SUBCLASS_ACTIVE.store(false, Ordering::Release);
    apply();
}

/// Records whether presented frames are copied from the render GPU to a
/// different display GPU, which turns the identical-frame skip off.
pub fn set_cross_adapter_presentation(cross_adapter: bool) {
    CROSS_ADAPTER.store(cross_adapter, Ordering::Release);
    apply();
}

/// Invalidates the presented frame for messages that mean the window manager
/// may show stale or discarded content.
pub(super) fn observe(hwnd: HWND, msg: u32) {
    let invalidates = match msg {
        WM_SIZE | WM_SHOWWINDOW | WM_DISPLAYCHANGE | WM_DPICHANGED => true,
        // winit's own redraw requests post an internal paint with no update
        // region; only a real invalid region means the OS needs new pixels.
        WM_PAINT => unsafe { GetUpdateRect(hwnd, None, false) }.as_bool(),
        _ => false,
    };
    if invalidates {
        eframe::egui_wgpu::winit::invalidate_presented_frames();
    }
}
