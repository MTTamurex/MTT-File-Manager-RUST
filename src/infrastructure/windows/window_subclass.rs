//! Window subclass for borderless resize handling on Windows
//!
//! This module provides native Windows message handling for borderless windows.
//! It intercepts WM_NCHITTEST to provide resize borders when decorations are disabled.
//!
//! # Why This Is Required
//! - egui/winit cannot handle native window chrome
//! - winit returns HTCLIENT for entire borderless window
//! - Windows needs edge codes (HTLEFT, HTRIGHT, etc.) for resize cursors/behavior

use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsZoomed, WM_NCHITTEST, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
};

/// Resize border width in pixels (scaled by DPI at runtime if needed)
const RESIZE_BORDER_WIDTH: i32 = 8;

/// Subclass ID for our borderless handler
const BORDERLESS_SUBCLASS_ID: usize = 1;

/// Flag to track if subclass is installed (prevents double-install)
static SUBCLASS_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Install the borderless window subclass on the given HWND.
///
/// This intercepts WM_NCHITTEST to provide resize borders.
/// Safe to call multiple times - will only install once.
///
/// # Safety
/// HWND must be a valid window handle from the current process.
pub fn install_borderless_subclass(hwnd: HWND) -> bool {
    if SUBCLASS_INSTALLED.load(Ordering::SeqCst) {
        return true; // Already installed
    }

    // SAFETY: We pass a valid HWND, a valid callback, and our subclass ID.
    // The callback follows Windows calling convention via `extern "system"`.
    let result = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(borderless_subclass_proc),
            BORDERLESS_SUBCLASS_ID,
            0, // No extra data needed
        )
    };

    if result.as_bool() {
        SUBCLASS_INSTALLED.store(true, Ordering::SeqCst);
        true
    } else {
        eprintln!("Failed to install borderless window subclass");
        false
    }
}

/// Remove the borderless window subclass.
///
/// Call this on window close to clean up.
pub fn remove_borderless_subclass(hwnd: HWND) {
    if !SUBCLASS_INSTALLED.load(Ordering::SeqCst) {
        return;
    }

    // SAFETY: HWND is valid, we're removing our own subclass.
    unsafe {
        let _ = RemoveWindowSubclass(hwnd, Some(borderless_subclass_proc), BORDERLESS_SUBCLASS_ID);
    }

    SUBCLASS_INSTALLED.store(false, Ordering::SeqCst);
}

/// Subclass window procedure that handles WM_NCHITTEST for borderless resize.
///
/// # Message Handling
/// - WM_NCHITTEST: Returns edge/corner codes for resize zones, HTCLIENT otherwise
/// - All other messages: Passed to DefSubclassProc
extern "system" fn borderless_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid_subclass: usize,
    _dw_ref_data: usize,
) -> LRESULT {
    if msg == WM_NCHITTEST {
        return handle_nchittest(hwnd, lparam);
    }

    // Pass all other messages to default handler
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Handle WM_NCHITTEST message to provide resize borders.
///
/// Returns appropriate hit-test code based on cursor position:
/// - Edge zones (8px from border): HTLEFT, HTRIGHT, HTTOP, HTBOTTOM
/// - Corner zones (8x8px): HTTOPLEFT, HTTOPRIGHT, HTBOTTOMLEFT, HTBOTTOMRIGHT
/// - Rest of window: HTCLIENT (let egui handle)
fn handle_nchittest(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    // Don't provide resize borders when maximized
    // SAFETY: HWND is valid, IsZoomed just queries window state
    if unsafe { IsZoomed(hwnd).as_bool() } {
        return LRESULT(HTCLIENT as isize);
    }

    // Extract cursor position from lparam (screen coordinates)
    let cursor_x = (lparam.0 as i32) & 0xFFFF;
    let cursor_y = ((lparam.0 as i32) >> 16) & 0xFFFF;

    // Handle signed coordinate conversion for multi-monitor setups
    let cursor_x = if cursor_x > 32767 {
        cursor_x - 65536
    } else {
        cursor_x
    };
    let cursor_y = if cursor_y > 32767 {
        cursor_y - 65536
    } else {
        cursor_y
    };

    // Get window client rect
    let mut client_rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: HWND is valid, client_rect is a valid mutable reference
    if unsafe { GetClientRect(hwnd, &mut client_rect).is_err() } {
        return LRESULT(HTCLIENT as isize);
    }

    // Convert screen coords to window-relative coords
    // We need window rect, not client rect, for proper edge detection
    let mut window_rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: HWND is valid
    if unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut window_rect).is_err()
    } {
        return LRESULT(HTCLIENT as isize);
    }

    // Calculate position relative to window
    let x = cursor_x - window_rect.left;
    let y = cursor_y - window_rect.top;
    let width = window_rect.right - window_rect.left;
    let height = window_rect.bottom - window_rect.top;

    // Check if cursor is within window bounds
    if x < 0 || y < 0 || x >= width || y >= height {
        return LRESULT(HTCLIENT as isize);
    }

    // Determine hit-test zone
    let on_left = x < RESIZE_BORDER_WIDTH;
    let on_right = x >= width - RESIZE_BORDER_WIDTH;
    let on_top = y < RESIZE_BORDER_WIDTH;
    let on_bottom = y >= height - RESIZE_BORDER_WIDTH;

    // Corner detection (corners take priority)
    let hit_test = if on_top && on_left {
        HTTOPLEFT
    } else if on_top && on_right {
        HTTOPRIGHT
    } else if on_bottom && on_left {
        HTBOTTOMLEFT
    } else if on_bottom && on_right {
        HTBOTTOMRIGHT
    } else if on_left {
        HTLEFT
    } else if on_right {
        HTRIGHT
    } else if on_top {
        HTTOP
    } else if on_bottom {
        HTBOTTOM
    } else {
        HTCLIENT
    };

    LRESULT(hit_test as isize)
}

/// RAII guard for borderless subclass.
///
/// Automatically removes subclass when dropped.
pub struct BorderlessSubclass {
    hwnd: HWND,
}

impl BorderlessSubclass {
    /// Create and install borderless subclass.
    ///
    /// Returns None if installation fails.
    pub fn new(hwnd: HWND) -> Option<Self> {
        if install_borderless_subclass(hwnd) {
            Some(Self { hwnd })
        } else {
            None
        }
    }
}

impl Drop for BorderlessSubclass {
    fn drop(&mut self) {
        remove_borderless_subclass(self.hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resize_border_constant() {
        // Ensure border width is reasonable
        assert!(RESIZE_BORDER_WIDTH > 0);
        assert!(RESIZE_BORDER_WIDTH <= 20);
    }
}
