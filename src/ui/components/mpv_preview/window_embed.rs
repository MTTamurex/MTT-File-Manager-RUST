use super::*;

impl MpvPreview {
    #[cfg(target_os = "windows")]
    pub fn release_focus(&self, _main_hwnd: HWND) {
        // MPV does not capture focus by default.
    }

    #[cfg(target_os = "windows")]
    pub fn release_focus_auto(&self) {
        // No-op for MPV. Keep for API parity.
    }

    #[cfg(target_os = "windows")]
    pub fn has_hwnd(&self, hwnd: HWND) -> bool {
        self.mpv_hwnd == Some(hwnd)
    }

    #[cfg(target_os = "windows")]
    pub fn get_hwnd(&self) -> Option<HWND> {
        self.mpv_hwnd
    }

    #[cfg(target_os = "windows")]
    pub(super) fn ensure_main_hwnd_from_frame(&mut self, frame: Option<&eframe::Frame>) {
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

    #[cfg(not(target_os = "windows"))]
    pub(super) fn ensure_main_hwnd_from_frame(&mut self, _frame: Option<&eframe::Frame>) {}

    #[cfg(target_os = "windows")]
    pub(super) fn ensure_mpv_hwnd_child(&mut self) {
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
                if let Some(m) = &self.mpv {
                    let _ = m.set_property("wid", h_video.0 as i64);
                }
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    pub(super) fn ensure_mpv_hwnd_child(&mut self) {}

    #[cfg(target_os = "windows")]
    pub(super) fn sync_child_window_rect(&mut self, ui: &egui::Ui, rect: egui::Rect) {
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

    #[cfg(not(target_os = "windows"))]
    pub(super) fn sync_child_window_rect(&mut self, _ui: &egui::Ui, rect: egui::Rect) {
        if rect != self.last_rect {
            self.last_rect = rect;
        }
    }
}
