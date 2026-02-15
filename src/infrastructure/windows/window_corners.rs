//! Native window corner preferences (Windows 11 DWM).
//!
//! Keeps the app with rounded corners in windowed mode while preserving square
//! corners when maximized.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DWMWCP_ROUND,
};

/// Apply native DWM corner preference for the main window.
///
/// - `is_maximized = false`: rounded corners
/// - `is_maximized = true`: no rounding
pub fn apply_window_corner_preference(hwnd: HWND, is_maximized: bool) {
    if hwnd.is_invalid() {
        return;
    }

    let pref: DWM_WINDOW_CORNER_PREFERENCE = if is_maximized {
        DWMWCP_DONOTROUND
    } else {
        DWMWCP_ROUND
    };

    // Best-effort: ignore failure on unsupported OS/configuration.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}
