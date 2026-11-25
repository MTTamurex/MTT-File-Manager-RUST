use eframe::egui;

#[cfg(target_os = "windows")]
use windows::core::w;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HWND, POINT};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn,
    UpdateWindow, RGN_DIFF,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, MoveWindow, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

#[cfg(target_os = "windows")]
fn create_surface_window(parent: HWND, popup: bool) -> windows::core::Result<HWND> {
    use windows::Win32::System::SystemServices::SS_BLACKRECT;

    // Popup experiment: an owned, never-activated top-level window. DWM then
    // composes the video like the detached player instead of re-composing the
    // main window at the video's cadence. `parent` becomes the owner.
    let (ex_style, style) = if popup {
        (
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            WS_POPUP | WS_CLIPCHILDREN,
        )
    } else {
        (
            WINDOW_EX_STYLE::default(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        )
    };
    // A default STATIC has no opaque video background. Keep the surface hidden
    // until it is positioned, and let STATIC paint black until MPV takes over.
    unsafe {
        CreateWindowExW(
            ex_style,
            w!("static"),
            w!(""),
            style | WINDOW_STYLE(SS_BLACKRECT.0),
            0,
            0,
            1,
            1,
            Some(parent),
            None,
            None,
            None,
        )
    }
}

/// Encapsulates all native window (HWND) management for the MPV video surface.
///
/// This struct isolates all platform-specific window operations so that the rest
/// of the codebase never needs to interact with HWND directly.
#[cfg(target_os = "windows")]
pub struct VideoSurface {
    mpv_hwnd: Option<HWND>,
    main_hwnd: Option<HWND>,
    last_rect: egui::Rect,
    /// Screen position of the main window's client origin (popup host only).
    last_client_origin: Option<(i32, i32)>,
    popup: bool,
    /// Visible part of a popup host inside the main client area, in window
    /// coordinates; `None` when the whole surface is inside.
    clip: Option<(i32, i32, i32, i32)>,
    last_overlay_regions: Option<Vec<PixelRoundedRect>>,
    visible: std::cell::Cell<bool>,
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

/// Part of a popup surface `(x, y, width, height)` (client coordinates) that
/// lies inside a client area of `client_size`, in surface coordinates.
/// `None` when nothing needs clipping.
#[cfg(any(target_os = "windows", test))]
fn popup_clip(
    (x, y, width, height): (i32, i32, i32, i32),
    (client_width, client_height): (i32, i32),
) -> Option<(i32, i32, i32, i32)> {
    let left = (-x).clamp(0, width);
    let top = (-y).clamp(0, height);
    let right = (client_width - x).clamp(left, width);
    let bottom = (client_height - y).clamp(top, height);
    (left != 0 || top != 0 || right != width || bottom != height)
        .then_some((left, top, right, bottom))
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
            last_client_origin: None,
            popup: super::host_tuning::PreviewHostTuning::from_env().popup,
            clip: None,
            last_overlay_regions: None,
            visible: std::cell::Cell::new(false),
        }
    }

    /// Screen position of the main window's client area (popup host only; a
    /// child window is positioned in client coordinates).
    fn client_origin(&self) -> Option<(i32, i32)> {
        if !self.popup {
            return Some((0, 0));
        }
        let main = self.main_hwnd?;
        let mut origin = POINT { x: 0, y: 0 };
        unsafe { ClientToScreen(main, &mut origin) }
            .as_bool()
            .then_some((origin.x, origin.y))
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

        match create_surface_window(parent, self.popup) {
            Ok(h_video) => {
                self.mpv_hwnd = Some(h_video);
                self.reset_rect();
                let _ = mpv.set_property("wid", h_video.0 as i64);
            }
            Err(error) => log::warn!("[MpvPreview] Failed to create video surface: {error}"),
        }
    }

    /// Synchronizes the child window position/size with the egui allocated rect.
    /// Only calls MoveWindow when position/size actually changes (~95% reduction).
    pub fn sync_rect(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        // A popup must also follow the main window when it moves on screen.
        let origin = self.client_origin();
        if rect == self.last_rect && origin == self.last_client_origin {
            return;
        }

        if rect != self.last_rect {
            self.last_overlay_regions = None;
        }
        self.last_rect = rect;
        self.last_client_origin = origin;

        if let Some(h_video) = self.mpv_hwnd {
            let (origin_x, origin_y) = origin.unwrap_or((0, 0));
            let factor = ui.ctx().pixels_per_point();
            let client_x = (rect.min.x * factor) as i32;
            let client_y = (rect.min.y * factor) as i32;
            let w = (rect.width() * factor) as i32;
            let h = (rect.height() * factor) as i32;
            if self.popup {
                crate::infrastructure::windows::owned_surface::set_offset(
                    h_video,
                    (client_x, client_y),
                );
                // A child was clipped to the client area by Windows; a popup
                // must be clipped explicitly (e.g. a floating player dragged
                // partly outside the main window).
                let clip = self.main_hwnd.and_then(|main| {
                    let mut client = windows::Win32::Foundation::RECT::default();
                    unsafe {
                        windows::Win32::UI::WindowsAndMessaging::GetClientRect(main, &mut client)
                    }
                    .ok()?;
                    popup_clip(
                        (client_x, client_y, w.max(1), h.max(1)),
                        (client.right, client.bottom),
                    )
                });
                if clip != self.clip {
                    self.clip = clip;
                    self.last_overlay_regions = None;
                }
            }
            unsafe {
                // PERF: MoveWindow only called when position/size changes (~95% reduction)
                let _ = MoveWindow(
                    h_video,
                    client_x + origin_x,
                    client_y + origin_y,
                    w.max(1),
                    h.max(1),
                    true,
                );
            }
        }
    }

    /// Shows or hides the video surface.
    /// Use this to resolve Z-order issues when popups need to appear over the video area.
    pub fn set_visible(&self, visible: bool) {
        // Visibility may be requested before creation or before layout is ready.
        let visible = visible && self.last_rect.is_positive();
        if let Some(hwnd) = self.mpv_hwnd {
            if self.visible.replace(visible) != visible {
                // While the main window is hidden (tray), only record the wish;
                // the owner's WM_SHOWWINDOW shows the popup again.
                if self.popup
                    && !crate::infrastructure::windows::owned_surface::set_wants_visible(
                        hwnd, visible,
                    )
                {
                    return;
                }
                unsafe {
                    let _ = ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
                    if visible {
                        // Paint the fallback now, not in a later message-loop iteration.
                        let _ = UpdateWindow(hwnd);
                    }
                }
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
            if regions.is_empty() && self.clip.is_none() {
                SetWindowRgn(hwnd, None, true) != 0
            } else {
                let width = ((self.last_rect.width() * pixels_per_point) as i32).max(1);
                let height = ((self.last_rect.height() * pixels_per_point) as i32).max(1);
                let (left, top, right, bottom) = self.clip.unwrap_or((0, 0, width, height));
                let window_region = CreateRectRgn(left, top, right, bottom);
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
        self.visible.set(false);
        if let Some(hwnd) = self.mpv_hwnd.take() {
            crate::infrastructure::windows::owned_surface::untrack(hwnd);
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
#[path = "window_embed_tests.rs"]
mod tests;
