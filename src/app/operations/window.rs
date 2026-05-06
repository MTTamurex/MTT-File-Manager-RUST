//! Window handle management
//!
//! This module captures and stores the native window handle (HWND)
//! and performs initialization tasks that require it.

use crate::app::state::ImageViewerApp;
use crate::infrastructure::shell_menu_worker::ShellMenuRequest;
use crate::infrastructure::windows::window_corners::apply_window_corner_preference;
use crate::infrastructure::windows::window_subclass::install_borderless_subclass;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, FindWindowW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

/// Creates a hidden, unowned top-level popup window used as the owner for Shell
/// file-operation progress dialogs (copy, move, delete).
///
/// By giving Shell dialogs this invisible proxy as their owner instead of the
/// real app window, we prevent the Shell from disabling the app window while a
/// long operation (e.g. a large move/copy) is running or being cancelled.
/// The proxy has no owner (`hwndParent = None`), so disabling it does not
/// cascade to the app window.
///
/// The window is 0×0, never shown, and excluded from the taskbar/Alt+Tab
/// via `WS_EX_TOOLWINDOW`. It lives for the entire process lifetime.
fn create_shell_op_proxy_window() -> Option<HWND> {
    // "STATIC" is a built-in Windows class that requires no prior registration.
    let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();

    // SAFETY: CreateWindowExW is called with a valid null-terminated class name,
    // WS_POPUP style, zero size/position, and no owner/parent. The returned HWND
    // is valid for the entire process lifetime.
    unsafe {
        match CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,  // no owner — Shell only disables this window, not the app
            None,
            None,
            None,
        ) {
            Ok(h) if !h.is_invalid() => Some(h),
            _ => None,
        }
    }
}

impl ImageViewerApp {
    /// Returns the HWND to use as owner for Shell file-operation dialogs
    /// (copy, move, delete, rename). Prefers the invisible proxy window,
    /// falling back to the real app window if the proxy was not created.
    ///
    /// Using the proxy prevents the Shell from disabling the app's main window
    /// during long or cancelled file operations.
    pub fn shell_op_hwnd(&self) -> HWND {
        self.shell_op_proxy_hwnd
            .or(self.native_hwnd)
            .unwrap_or_default()
    }

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

                    // Create the invisible proxy window used as owner for Shell
                    // file-operation dialogs so the Shell cannot disable the app's
                    // main window during long or cancelled operations.
                    self.shell_op_proxy_hwnd = create_shell_op_proxy_window();
                    if self.shell_op_proxy_hwnd.is_none() {
                        log::warn!(
                            "Shell op proxy window creation failed; \
                             Shell dialogs may disable the app window during file ops"
                        );
                    }

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
