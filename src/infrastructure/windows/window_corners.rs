//! Native window DWM preferences (Windows 11).
//!
//! - Corner rounding: rounded in windowed mode, square when maximized.
//! - Title-bar dark mode via `DWMWA_USE_IMMERSIVE_DARK_MODE`.
//! - Optional caption/text colors for custom app palettes on Windows 11.

use windows::core::{Error, Result};
use windows::Win32::Foundation::{E_HANDLE, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DWMWCP_ROUND, DWMWINDOWATTRIBUTE, DWM_WINDOW_CORNER_PREFERENCE,
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
/// Windows 10 build 18985+; returns an error when unsupported.
pub fn apply_dark_title_bar(hwnd: HWND, dark: bool) -> Result<()> {
    if hwnd.is_invalid() {
        return Err(Error::from_hresult(E_HANDLE));
    }

    // DWMWA_USE_IMMERSIVE_DARK_MODE = 20
    const DWMWA_USE_IMMERSIVE_DARK_MODE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(20);
    let value: i32 = if dark { 1 } else { 0 };

    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<i32>() as u32,
        )
    }
}

fn colorref([r, g, b]: [u8; 3]) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// Set custom caption/text colors, or restore Windows defaults with `None`.
/// These attributes require Windows 11 build 22000 or newer.
pub fn apply_title_bar_colors(hwnd: HWND, colors: Option<([u8; 3], [u8; 3])>) -> Result<()> {
    if hwnd.is_invalid() {
        return Err(Error::from_hresult(E_HANDLE));
    }
    const DWMWA_COLOR_DEFAULT: u32 = 0xffff_ffff;
    let (caption, text) = colors.map_or((DWMWA_COLOR_DEFAULT, DWMWA_COLOR_DEFAULT), |(bg, fg)| {
        (colorref(bg), colorref(fg))
    });
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<u32>() as u32,
        )
        .map_err(|error| Error::new(error.code(), "DWMWA_CAPTION_COLOR"))?;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<u32>() as u32,
        )
        .map_err(|error| Error::new(error.code(), "DWMWA_TEXT_COLOR"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_colors_use_windows_colorref_byte_order() {
        assert_eq!(colorref([0x0d, 0x11, 0x17]), 0x0017_110d);
        assert_eq!(colorref([0xe6, 0xed, 0xf3]), 0x00f3_ede6);
    }

    #[test]
    fn invalid_native_windows_report_errors_instead_of_succeeding_silently() {
        let hwnd = HWND::default();
        assert_eq!(
            apply_dark_title_bar(hwnd, true).unwrap_err().code(),
            E_HANDLE
        );
        assert_eq!(
            apply_title_bar_colors(hwnd, Some(([13, 17, 23], [230, 237, 243])))
                .unwrap_err()
                .code(),
            E_HANDLE
        );
    }
}
