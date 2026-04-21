//! Native window DWM preferences (Windows 11).
//!
//! - Corner rounding: rounded in windowed mode, square when maximized.
//! - Title-bar dark mode via `DWMWA_USE_IMMERSIVE_DARK_MODE`.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWINDOWATTRIBUTE, DWM_WINDOW_CORNER_PREFERENCE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
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

/// Apply or remove the immersive dark-mode flag on the native title bar.
///
/// Uses `DWMWA_USE_IMMERSIVE_DARK_MODE` (attribute 20). Requires
/// Windows 10 build 18985+; silently ignored on older versions.
pub fn apply_dark_title_bar(hwnd: HWND, dark: bool) {
    if hwnd.is_invalid() {
        return;
    }

    // DWMWA_USE_IMMERSIVE_DARK_MODE = 20
    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
    let value: i32 = if dark { 1 } else { 0 };

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<i32>() as u32,
        );
    }
}

/// Disable native DWM show/hide transitions for a window.
///
/// This is useful for viewers that must appear instantly without composition
/// animations that can look like rapid extra window opens/closes.
pub fn disable_window_transitions(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }

    // DWMWA_TRANSITIONS_FORCEDISABLED = 3
    const DWMWA_TRANSITIONS_FORCEDISABLED: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(3);
    let value: i32 = 1;

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &value as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<i32>() as u32,
        );
    }
}
