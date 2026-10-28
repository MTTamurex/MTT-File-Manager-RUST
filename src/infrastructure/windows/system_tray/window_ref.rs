use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetPropW, GetWindowThreadProcessId};

/// Menu snapshots must not target a recycled HWND belonging to another process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowRef {
    pub hwnd: HWND,
    pid: u32,
}

impl WindowRef {
    pub(super) fn process_id(self) -> u32 {
        self.pid
    }
    pub(super) fn capture(hwnd: HWND) -> Option<Self> {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0
                || GetPropW(hwnd, w!("MTT.FileManager.MainWindow")).is_invalid()
                || !GetPropW(hwnd, w!("MTT.FileManager.Exiting")).is_invalid()
            {
                return None;
            }
        }
        Some(Self { hwnd, pid })
    }

    pub(super) fn is_current(self) -> bool {
        Self::capture(self.hwnd) == Some(self)
    }
}
