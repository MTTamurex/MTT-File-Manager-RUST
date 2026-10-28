use std::mem::size_of;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI};

pub(super) const WIDTH: i32 = 340;
pub(super) const HEADER: i32 = 52;
pub(super) const FOOTER_ITEM_HEIGHT: i32 = 24;
pub(super) const FOOTER: i32 = 2 * FOOTER_ITEM_HEIGHT;

pub(super) struct Layout {
    pub dpi: u32,
    pub bounds: RECT,
    pub width: i32,
    pub height: i32,
}

impl Layout {
    pub fn new(owner: HWND, point: POINT, content: i32) -> Option<Self> {
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return None;
        }
        let mut dpi = unsafe { GetDpiForWindow(owner) }.max(96);
        let mut x = 0;
        let mut y = 0;
        if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) }.is_ok() && x != 0
        {
            dpi = x;
        }
        Some(Self::within(
            info.rcWork,
            point,
            dpi.clamp(48, 768),
            content,
        ))
    }

    fn within(work: RECT, point: POINT, dpi: u32, content: i32) -> Self {
        let scale = |value: i32| ((value as i64 * dpi as i64 + 48) / 96) as i32;
        let width = scale(WIDTH).min((work.right - work.left - scale(16)).max(1));
        let height = scale((content + HEADER + FOOTER).min(480))
            .min((work.bottom - work.top - scale(16)).max(1));
        let x = (point.x - width).clamp(work.left, (work.right - width).max(work.left));
        let y = (point.y - height).clamp(work.top, (work.bottom - height).max(work.top));
        Self {
            dpi,
            bounds: RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            },
            width: width * 96 / dpi as i32,
            height: height * 96 / dpi as i32,
        }
    }

    pub fn px(&self, value: i32) -> i32 {
        ((value as i64 * self.dpi as i64 + 48) / 96) as i32
    }
    pub fn logical(&self, value: i32) -> i32 {
        (value * 96).div_euclid(self.dpi as i32)
    }
    pub fn viewport(&self) -> i32 {
        (self.height - HEADER - FOOTER).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_fits_negative_coordinate_monitors_at_multiple_scales() {
        let work = RECT {
            left: -1920,
            top: -200,
            right: 0,
            bottom: 840,
        };
        for dpi in [96, 144, 192, 288] {
            for point in [POINT { x: -1920, y: -200 }, POINT { x: -5, y: 835 }] {
                let layout = Layout::within(work, point, dpi, 10000);
                assert!(layout.bounds.left >= work.left && layout.bounds.right <= work.right);
                assert!(layout.bounds.top >= work.top && layout.bounds.bottom <= work.bottom);
                assert!(layout.viewport() > 0);
                assert!(layout.logical(-1) < 0);
            }
        }
    }

    #[test]
    fn compact_popup_keeps_a_single_instance_small_and_caps_expanded_height() {
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let point = POINT { x: 1900, y: 1060 };
        for dpi in [96, 144, 192] {
            let collapsed = Layout::within(work, point, dpi, 52);
            assert_eq!(collapsed.width, 340);
            assert_eq!(collapsed.height, 152);
            let expanded = Layout::within(work, point, dpi, 10000);
            assert_eq!(expanded.height, 480);
        }
    }
}
