use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

/// Center a window on the primary monitor's work area (excluding taskbar)
/// using the window's current outer size.  This is called once after the
/// native HWND is known so the calculation uses real screen pixels and
/// includes the decoration frame, avoiding DPI/logical-unit mismatches.
pub fn center_window_on_primary_monitor(hwnd: HWND) {
    unsafe {
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: Default::default(),
            rcWork: Default::default(),
            dwFlags: 0,
        };
        if !GetMonitorInfoW(hmonitor, &mut info).as_bool() {
            return;
        }

        let mut wnd_rect = RECT::default();
        if GetWindowRect(hwnd, &mut wnd_rect).is_err() {
            return;
        }

        let win_width = wnd_rect.right - wnd_rect.left;
        let win_height = wnd_rect.bottom - wnd_rect.top;

        let work_x = info.rcWork.left;
        let work_y = info.rcWork.top;
        let work_width = info.rcWork.right - info.rcWork.left;
        let work_height = info.rcWork.bottom - info.rcWork.top;

        let x = work_x + (work_width - win_width) / 2;
        let y = work_y + (work_height - win_height) / 2;

        // Clamp so the window never overflows outside the work area.
        let x = x.max(work_x);
        let y = y.max(work_y);

        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}
