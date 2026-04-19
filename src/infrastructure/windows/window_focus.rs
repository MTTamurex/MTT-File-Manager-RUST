use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

#[inline]
pub fn is_foreground_window(hwnd: HWND) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    !foreground.is_invalid() && foreground == hwnd
}