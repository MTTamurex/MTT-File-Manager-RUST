use eframe::egui;

#[cfg(any(target_os = "windows", test))]
use crate::infrastructure::windows::surface_region::Cutout;
#[cfg(target_os = "windows")]
use crate::infrastructure::windows::{owned_surface, surface_region};
#[cfg(target_os = "windows")]
use windows::core::w;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{HWND, POINT};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::{ClientToScreen, UpdateWindow};
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
    /// Anchor and client size (physical pixels) of the last popup layout.
    last_placement: Option<(owned_surface::Anchor, (i32, i32))>,
    popup: bool,
    last_overlay_regions: Option<Vec<Cutout>>,
    visible: std::cell::Cell<bool>,
}

#[cfg(not(target_os = "windows"))]
pub struct VideoSurface {
    last_rect: egui::Rect,
}

#[cfg(any(target_os = "windows", test))]
fn rounded_rect_to_pixels(
    rect: egui::Rect,
    corner_radius: f32,
    video_left: i32,
    video_top: i32,
    pixels_per_point: f32,
) -> Cutout {
    Cutout {
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
) -> Vec<Cutout> {
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
            last_placement: None,
            popup: super::host_tuning::PreviewHostTuning::from_env().popup,
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

    /// Synchronizes the native window position/size with the egui allocated
    /// rect. `anchor` says how a popup host follows the main window's client
    /// area when it changes size before the next layout.
    pub fn sync_rect(&mut self, ui: &egui::Ui, rect: egui::Rect, anchor: owned_surface::Anchor) {
        let factor = ui.ctx().pixels_per_point();
        // A popup must also follow the main window when it moves on screen.
        let origin = self.client_origin();
        let viewport = ui.ctx().viewport_rect();
        let client_size = (
            (viewport.width() * factor).round() as i32,
            (viewport.height() * factor).round() as i32,
        );
        let placement = self.popup.then_some((anchor, client_size));
        if rect == self.last_rect
            && origin == self.last_client_origin
            && placement == self.last_placement
        {
            return;
        }

        if rect != self.last_rect {
            self.last_overlay_regions = None;
        }
        self.last_rect = rect;
        self.last_client_origin = origin;
        self.last_placement = placement;

        let Some(h_video) = self.mpv_hwnd else {
            return;
        };
        let position = ((rect.min.x * factor) as i32, (rect.min.y * factor) as i32);
        let size = (
            ((rect.width() * factor) as i32).max(1),
            ((rect.height() * factor) as i32).max(1),
        );
        match (self.popup, self.main_hwnd) {
            (true, Some(main)) => owned_surface::place(
                h_video,
                main,
                owned_surface::Layout {
                    position,
                    size,
                    client_size,
                    anchor,
                },
            ),
            // A child is positioned in client coordinates and clipped by Windows.
            _ => unsafe {
                let _ = MoveWindow(h_video, position.0, position.1, size.0, size.1, true);
            },
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

        // A popup's region also carries its clip, which changes with the main
        // window: the registry owns it.
        let applied = if self.popup {
            owned_surface::set_cutouts(hwnd, regions.clone());
            true
        } else {
            let size = (
                (self.last_rect.width() * pixels_per_point) as i32,
                (self.last_rect.height() * pixels_per_point) as i32,
            );
            surface_region::apply(hwnd, size, None, &regions)
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

    pub fn sync_rect(
        &mut self,
        _ui: &egui::Ui,
        rect: egui::Rect,
        _anchor: crate::infrastructure::windows::owned_surface::Anchor,
    ) {
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
