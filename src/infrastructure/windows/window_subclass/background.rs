//! Tracks whether the main window is out of view (minimized or hidden to the
//! system tray) for background memory maintenance.
//!
//! Hiding to the tray uses `SW_HIDE`, which never reaches the minimized layout
//! phase. The hidden window is remembered and its visibility is queried live,
//! so every re-show path (tray, external open, other instances) is covered
//! without having to clear a flag.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

static TRAY_HIDDEN_HWND: AtomicIsize = AtomicIsize::new(0);

/// Records the window that is being hidden to the system tray.
pub fn mark_hidden_to_tray(hwnd: HWND) {
    TRAY_HIDDEN_HWND.store(hwnd.0 as isize, Ordering::Release);
}

fn is_hidden_to_tray() -> bool {
    let hwnd = TRAY_HIDDEN_HWND.load(Ordering::Acquire);
    hwnd != 0 && !unsafe { IsWindowVisible(HWND(hwnd as _)) }.as_bool()
}

fn backgrounded(minimized: bool, hidden_to_tray: impl FnOnce() -> bool) -> bool {
    minimized || hidden_to_tray()
}

/// `true` while the main window is minimized or hidden to the system tray.
pub fn is_backgrounded() -> bool {
    backgrounded(super::is_minimized(), is_hidden_to_tray)
}

#[cfg(test)]
mod tests {
    use super::backgrounded;

    #[test]
    fn minimized_or_tray_hidden_windows_are_backgrounded() {
        assert!(backgrounded(true, || false));
        assert!(backgrounded(false, || true));
        assert!(!backgrounded(false, || false));
    }
}
