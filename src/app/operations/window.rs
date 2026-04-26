//! Window handle management
//!
//! This module captures and stores the native window handle (HWND)
//! and performs initialization tasks that require it.

use crate::app::state::ImageViewerApp;
use crate::infrastructure::shell_menu_worker::ShellMenuRequest;
use crate::infrastructure::windows::window_corners::apply_window_corner_preference;
use crate::infrastructure::windows::window_subclass::install_borderless_subclass;
use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

impl ImageViewerApp {
    /// Captures and stores the native HWND from the main window title.
    /// On first capture, also warms up shell extensions to avoid
    /// slowness on the first context menu opening.
    ///
    /// # Borderless Window Support
    /// When HWND is obtained, installs a native subclass to handle WM_NCHITTEST
    /// for resize borders on the borderless window.
    pub fn ensure_window_handle(&mut self, _frame: &eframe::Frame) {
        if self.native_hwnd.is_some() {
            return;
        }

        // Try to find the window by title
        // Note: This is a hack because eframe doesn't yet expose HWND directly in a safe/easy way on Windows
        // The title must match the one defined in main.rs
        let window_title = "MTT File Manager\0".encode_utf16().collect::<Vec<u16>>();

        unsafe {
            if let Ok(hwnd) = FindWindowW(None, PCWSTR(window_title.as_ptr())) {
                if !hwnd.is_invalid() {
                    self.native_hwnd = Some(hwnd);

                    // Install borderless subclass for resize borders
                    // This handles WM_NCHITTEST to provide resize zones on window edges
                    if install_borderless_subclass(hwnd) {
                        log::info!("Borderless resize subclass installed successfully");
                    } else {
                        log::warn!("Failed to install borderless resize subclass");
                    }

                    // Keep rounded corners in windowed mode (Windows 11 DWM).
                    apply_window_corner_preference(hwnd, self.layout.saved_is_maximized);

                    // Warm shell extensions on the managed STA worker thread.
                    // This restores first-open context menu UX without spawning
                    // an unmanaged background thread.
                    let _ = self.shell_menu_req_tx.send(ShellMenuRequest::Warmup {
                        hwnd_isize: hwnd.0 as isize,
                    });
                }
            }
        }
    }
}
