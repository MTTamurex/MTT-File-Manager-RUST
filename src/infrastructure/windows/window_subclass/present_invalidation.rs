//! Tells the renderer when the main window needs a freshly presented frame.
//!
//! The vendored `egui-wgpu` skips rendering and presenting a frame identical to
//! the one on screen, so an idle window costs no GPU time. DWM does not keep
//! that image across every window-manager transition (minimize discards it),
//! so the skip is enabled only while this subclass reports those transitions.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::GetUpdateRect;
use windows::Win32::UI::WindowsAndMessaging::{
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_PAINT, WM_SHOWWINDOW, WM_SIZE,
};

pub(super) fn enable() {
    eframe::egui_wgpu::winit::set_identical_frame_skip(true);
}

pub(super) fn disable() {
    eframe::egui_wgpu::winit::set_identical_frame_skip(false);
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
