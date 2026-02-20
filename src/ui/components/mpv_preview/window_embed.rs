use eframe::egui;

#[cfg(target_os = "windows")]
use windows::core::w;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, CW_USEDEFAULT, SW_HIDE, SW_SHOW,
    WINDOW_EX_STYLE, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// Encapsulates all native window (HWND) management for the MPV video surface.
///
/// This struct isolates all platform-specific window operations so that the rest
/// of the codebase never needs to interact with HWND directly.
#[cfg(target_os = "windows")]
pub struct VideoSurface {
    mpv_hwnd: Option<HWND>,
    main_hwnd: Option<HWND>,
    last_rect: egui::Rect,
}

#[cfg(not(target_os = "windows"))]
pub struct VideoSurface {
    last_rect: egui::Rect,
}

#[cfg(target_os = "windows")]
impl VideoSurface {
    pub fn new() -> Self {
        Self {
            mpv_hwnd: None,
            main_hwnd: None,
            last_rect: egui::Rect::NAN,
        }
    }

    /// Captures the main application HWND from the eframe::Frame (called once).
    pub fn ensure_main_hwnd(&mut self, frame: Option<&eframe::Frame>) {
        if self.main_hwnd.is_some() {
            return;
        }

        let Some(frame) = frame else {
            return;
        };

        use raw_window_handle::HasWindowHandle;
        if let Ok(handle) = frame.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(wh) = handle.as_raw() {
                let hwnd = HWND(wh.hwnd.get() as _);
                if !hwnd.is_invalid() {
                    self.main_hwnd = Some(hwnd);
                }
            }
        }
    }

    /// Creates the child window for MPV rendering (called once).
    /// Sets the `wid` property on the MPV instance so it renders into this window.
    pub fn ensure_child_window(&mut self, mpv: &mpv::Mpv) {
        if self.mpv_hwnd.is_some() {
            return;
        }

        let Some(parent) = self.main_hwnd else {
            return;
        };

        unsafe {
            let h_video = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("static"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap_or(HWND::default());

            if !h_video.is_invalid() {
                self.mpv_hwnd = Some(h_video);
                let _ = mpv.set_property("wid", h_video.0 as i64);
            }
        }
    }

    /// Synchronizes the child window position/size with the egui allocated rect.
    /// Only calls MoveWindow when position/size actually changes (~95% reduction).
    pub fn sync_rect(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        if rect == self.last_rect {
            return;
        }

        self.last_rect = rect;

        if let Some(h_video) = self.mpv_hwnd {
            let factor = ui.ctx().pixels_per_point();
            let x = (rect.min.x * factor) as i32;
            let y = (rect.min.y * factor) as i32;
            let w = (rect.width() * factor) as i32;
            let h = (rect.height() * factor) as i32;
            unsafe {
                // PERF: MoveWindow only called when position/size changes (~95% reduction)
                let _ = MoveWindow(h_video, x, y, w.max(1), h.max(1), true);
            }
        }
    }

    /// Shows or hides the video surface.
    /// Use this to resolve Z-order issues when popups need to appear over the video area.
    pub fn set_visible(&self, visible: bool) {
        if let Some(hwnd) = self.mpv_hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
            }
        }
    }

    /// Returns the MPV child window HWND.
    pub fn hwnd(&self) -> Option<HWND> {
        self.mpv_hwnd
    }

    /// Returns the main application HWND.
    pub fn main_hwnd(&self) -> Option<HWND> {
        self.main_hwnd
    }

    /// Checks if the given HWND matches the MPV child window.
    pub fn has_hwnd(&self, hwnd: HWND) -> bool {
        self.mpv_hwnd == Some(hwnd)
    }

    /// Restores keyboard focus to the main application window if the MPV child
    /// window has captured it. This prevents the HWND from stealing keyboard
    /// shortcuts from egui.
    pub fn ensure_focus_on_main(&self) {
        if let (Some(mpv_h), Some(main_h)) = (self.mpv_hwnd, self.main_hwnd) {
            unsafe {
                if GetFocus() == mpv_h {
                    let _ = SetFocus(Some(main_h));
                }
            }
        }
    }

    /// Returns true if the child window has been created.
    pub fn is_initialized(&self) -> bool {
        self.mpv_hwnd.is_some()
    }

    /// Destroys the child window and releases resources.
    pub fn destroy(&mut self) {
        if let Some(hwnd) = self.mpv_hwnd.take() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    /// Resets the last rect to force a MoveWindow call on the next frame.
    pub fn reset_rect(&mut self) {
        self.last_rect = egui::Rect::NAN;
    }

    /// No-op for MPV. Kept for API parity.
    pub fn release_focus(&self, _main_hwnd: HWND) {
        // MPV does not capture focus by default.
    }

    /// No-op for MPV. Kept for API parity.
    pub fn release_focus_auto(&self) {
        // No-op for MPV.
    }
}

#[cfg(target_os = "windows")]
impl Default for VideoSurface {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_os = "windows"))]
impl VideoSurface {
    pub fn new() -> Self {
        Self {
            last_rect: egui::Rect::NAN,
        }
    }

    pub fn ensure_main_hwnd(&mut self, _frame: Option<&eframe::Frame>) {}

    pub fn ensure_child_window(&mut self, _mpv: &mpv::Mpv) {}

    pub fn sync_rect(&mut self, _ui: &egui::Ui, rect: egui::Rect) {
        if rect != self.last_rect {
            self.last_rect = rect;
        }
    }

    pub fn set_visible(&self, _visible: bool) {}

    pub fn hwnd(&self) -> Option<()> {
        None
    }

    pub fn main_hwnd(&self) -> Option<()> {
        None
    }

    pub fn ensure_focus_on_main(&self) {}

    pub fn is_initialized(&self) -> bool {
        false
    }

    pub fn destroy(&mut self) {}

    pub fn reset_rect(&mut self) {
        self.last_rect = egui::Rect::NAN;
    }
}
