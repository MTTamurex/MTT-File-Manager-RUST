use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic};

#[inline]
pub fn is_foreground_window(hwnd: HWND) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    !foreground.is_invalid() && foreground == hwnd
}

#[inline]
pub fn is_window_minimized(hwnd: HWND) -> bool {
    !hwnd.is_invalid() && unsafe { IsIconic(hwnd).as_bool() }
}
