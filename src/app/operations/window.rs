//! Window handle management
//!
//! This module captures and stores the native window handle (HWND)
//! and performs initialization tasks that require it.

use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::shell_operations::create_shell_op_proxy_window;
use crate::infrastructure::windows::window_corners::apply_window_corner_preference;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;

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

    /// Captures the native HWND belonging to this instance, not another window
    /// with the same title.
    /// # Borderless Window Support
    /// When HWND is obtained, installs a native subclass to handle WM_NCHITTEST
    /// for resize borders on the borderless window.
    pub fn ensure_window_handle(&mut self, frame: &eframe::Frame) {
        if self.native_hwnd.is_some() {
            return;
        }

        if let Ok(handle) = frame.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                let hwnd = HWND(handle.hwnd.get() as *mut _);
                self.native_hwnd = Some(hwnd);

                // The subclass also receives instance restoration/open requests.
                // Never advertise a window without that receiver, or leave a
                // failed startup alive holding the serialization mutex.
                if let Err(error) =
                    crate::infrastructure::windows::app_instances::initialize_main_window(hwnd)
                {
                    log::error!("[INSTANCES] Main window initialization failed: {error}");
                    self.tray_exit_requested = true;
                    self.ui_ctx
                        .send_viewport_cmd(eframe::egui::ViewportCommand::Close);
                    return;
                }

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

                // Keep rounded corners in windowed mode (Windows 11 DWM).
                apply_window_corner_preference(hwnd, self.layout.saved_is_maximized);
            }
        }
    }
}
