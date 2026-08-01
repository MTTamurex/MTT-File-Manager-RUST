use eframe::egui;

#[cfg(target_os = "windows")]
use windows::core::w;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, RGN_DIFF,
};
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
    last_overlay_regions: Option<Vec<PixelRoundedRect>>,
}

#[cfg(not(target_os = "windows"))]
pub struct VideoSurface {
    last_rect: egui::Rect,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PixelRoundedRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    corner_diameter: i32,
}

#[cfg(any(target_os = "windows", test))]
fn rounded_rect_to_pixels(
    rect: egui::Rect,
    corner_radius: f32,
    video_left: i32,
    video_top: i32,
    pixels_per_point: f32,
) -> PixelRoundedRect {
    PixelRoundedRect {
        left: (rect.left() * pixels_per_point).floor() as i32 - video_left,
        top: (rect.top() * pixels_per_point).floor() as i32 - video_top,
        right: (rect.right() * pixels_per_point).ceil() as i32 - video_left,
        bottom: (rect.bottom() * pixels_per_point).ceil() as i32 - video_top,
        corner_diameter: (corner_radius * 2.0 * pixels_per_point).round().max(1.0) as i32,
    }
}

#[cfg(any(target_os = "windows", test))]
fn overlay_pixel_regions(
    video_rect: egui::Rect,
    overlays: &[crate::ui::video_overlay::VideoOverlay],
    pixels_per_point: f32,
) -> Vec<PixelRoundedRect> {
    if !video_rect.is_positive() || pixels_per_point <= 0.0 {
        return Vec::new();
    }

    let video_left = (video_rect.left() * pixels_per_point).floor() as i32;
    let video_top = (video_rect.top() * pixels_per_point).floor() as i32;

    overlays
        .iter()
        .filter_map(|overlay| {
            if !video_rect.intersect(overlay.rect).is_positive() {
                return None;
            }

            Some(rounded_rect_to_pixels(
                overlay.rect,
                overlay.corner_radius,
                video_left,
                video_top,
                pixels_per_point,
            ))
        })
        .collect()
}

#[cfg(target_os = "windows")]
impl VideoSurface {
    pub fn new() -> Self {
        Self {
            mpv_hwnd: None,
            main_hwnd: None,
            last_rect: egui::Rect::NAN,
            last_overlay_regions: None,
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
        self.last_overlay_regions = None;

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

    /// Excludes egui overlay rectangles from the native video child window.
    /// The parent surface then remains visible and interactive in those areas.
    pub(crate) fn set_overlay_rects(
        &mut self,
        overlays: &[crate::ui::video_overlay::VideoOverlay],
        pixels_per_point: f32,
    ) {
        let Some(hwnd) = self.mpv_hwnd else {
            return;
        };

        let regions = overlay_pixel_regions(self.last_rect, overlays, pixels_per_point);
        if self.last_overlay_regions.as_ref() == Some(&regions) {
            return;
        }

        let applied = unsafe {
            if regions.is_empty() {
                SetWindowRgn(hwnd, None, true) != 0
            } else {
                let width = ((self.last_rect.width() * pixels_per_point) as i32).max(1);
                let height = ((self.last_rect.height() * pixels_per_point) as i32).max(1);
                let window_region = CreateRectRgn(0, 0, width, height);
                if window_region.is_invalid() {
                    false
                } else {
                    for rect in &regions {
                        let overlay_region = CreateRoundRectRgn(
                            rect.left,
                            rect.top,
                            rect.right,
                            rect.bottom,
                            rect.corner_diameter,
                            rect.corner_diameter,
                        );
                        if !overlay_region.is_invalid() {
                            let _ = CombineRgn(
                                Some(window_region),
                                Some(window_region),
                                Some(overlay_region),
                                RGN_DIFF,
                            );
                            let _ = DeleteObject(overlay_region.into());
                        }
                    }

                    if SetWindowRgn(hwnd, Some(window_region), true) != 0 {
                        // Windows owns window_region after a successful call.
                        true
                    } else {
                        let _ = DeleteObject(window_region.into());
                        false
                    }
                }
            }
        };

        if applied {
            self.last_overlay_regions = Some(regions);
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
        self.last_overlay_regions = None;
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

    pub(crate) fn set_overlay_rects(
        &mut self,
        _overlays: &[crate::ui::video_overlay::VideoOverlay],
        _pixels_per_point: f32,
    ) {
    }

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

#[cfg(test)]
mod tests {
    use super::{overlay_pixel_regions, PixelRoundedRect};
    use eframe::egui;

    fn popup_overlay(rect: egui::Rect) -> crate::ui::video_overlay::VideoOverlay {
        crate::ui::video_overlay::VideoOverlay {
            rect,
            corner_radius: 6.0,
        }
    }

    #[test]
    fn converts_overlay_intersection_to_video_local_pixels() {
        let video = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(300.0, 150.0));
        let overlay_rect =
            egui::Rect::from_min_max(egui::pos2(250.0, 25.0), egui::pos2(350.0, 100.0));

        assert_eq!(
            overlay_pixel_regions(video, &[popup_overlay(overlay_rect)], 1.5),
            vec![PixelRoundedRect {
                left: 225,
                top: -38,
                right: 375,
                bottom: 75,
                corner_diameter: 18,
            }]
        );
    }

    #[test]
    fn ignores_overlays_outside_video() {
        let video = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 100.0));
        let overlay_rect =
            egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(20.0, 20.0));

        assert!(overlay_pixel_regions(video, &[popup_overlay(overlay_rect)], 1.25).is_empty());
    }
}
