//! DWM cloaking for top-level windows.
//!
//! A cloaked window keeps its normal shown/maximized state (so the renderer
//! keeps painting it), but DWM does not compose it on screen. Used to hide
//! startup geometry changes until a frame exists at the final size.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};

/// Cloak or uncloak `hwnd`. Returns `false` when DWM rejects the request.
pub fn set_window_cloaked(hwnd: HWND, cloaked: bool) -> bool {
    if hwnd.is_invalid() {
        return false;
    }

    // DWMWA_CLOAK takes a Win32 BOOL.
    let value: i32 = i32::from(cloaked);
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<i32>() as u32,
        )
        .is_ok()
    }
}
