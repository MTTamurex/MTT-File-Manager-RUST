use super::labels;
use std::mem::size_of;
use windows::Win32::Foundation::{HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateFontIndirectW, DeleteDC, DeleteObject, GetMonitorInfoW,
    GetTextExtentPoint32W, MonitorFromPoint, SelectObject, HDC, HFONT, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, GetDpiForWindow, SystemParametersInfoForDpi, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS};

pub(super) struct MenuText {
    dc: HDC,
    font: HFONT,
    previous: HGDIOBJ,
    dpi: u32,
    budget: i32,
}

impl MenuText {
    pub(super) fn new(hwnd: HWND, point: POINT) -> Self {
        let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
        let mut dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let mut monitor_dpi = 0;
        let mut dpi_y = 0;
        if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut monitor_dpi, &mut dpi_y) }
            .is_ok()
            && monitor_dpi > 0
        {
            dpi = monitor_dpi;
        }
        dpi = dpi.clamp(48, 768);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let work_width = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            info.rcWork.right - info.rcWork.left
        } else {
            (1280 * dpi / 96) as i32
        };
        let dc = unsafe { CreateCompatibleDC(None) };
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        let font = if unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
                0,
                dpi,
            )
        }
        .is_ok()
        {
            unsafe { CreateFontIndirectW(&metrics.lfMenuFont) }
        } else {
            HFONT::default()
        };
        let previous = if dc.is_invalid() || font.is_invalid() {
            HGDIOBJ::default()
        } else {
            unsafe { SelectObject(dc, HGDIOBJ(font.0)) }
        };
        Self {
            dc,
            font,
            previous,
            dpi,
            budget: label_budget(work_width, dpi),
        }
    }

    fn measure(&self, value: &str) -> i32 {
        let wide: Vec<u16> = value.encode_utf16().collect();
        if wide.is_empty() {
            return 0;
        }
        let mut size = SIZE::default();
        if !self.dc.is_invalid()
            && !self.font.is_invalid()
            && !self.previous.is_invalid()
            && unsafe { GetTextExtentPoint32W(self.dc, &wide, &mut size) }.as_bool()
        {
            size.cx
        } else {
            // Conservative fallback if GDI resources are temporarily exhausted.
            (wide.len() as i32).saturating_mul((32 * self.dpi / 96) as i32)
        }
    }

    pub(super) fn fit(&self, value: &str) -> String {
        labels::compact(value, self.budget, &|s| self.measure(s))
    }

    pub(super) fn siblings<'a>(&self, values: impl Iterator<Item = &'a str>) -> Vec<String> {
        labels::siblings(&values.collect::<Vec<_>>(), self.budget, &|s| {
            self.measure(s)
        })
    }

    pub(super) fn caption(&self, prefix: &str, path: &str) -> String {
        let shortened = labels::compact(path, self.budget - self.measure(prefix), &|s| {
            self.measure(s)
        });
        if shortened.is_empty() && !path.is_empty() {
            self.fit(&format!("{prefix}{path}"))
        } else {
            self.fit(&format!("{prefix}{shortened}"))
        }
    }
}

impl Drop for MenuText {
    fn drop(&mut self) {
        unsafe {
            if !self.dc.is_invalid() {
                if !self.previous.is_invalid() {
                    let _ = SelectObject(self.dc, self.previous);
                }
                let _ = DeleteDC(self.dc);
            }
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}

fn label_budget(work_width: i32, dpi: u32) -> i32 {
    // Leave room for checkmarks, submenu arrows and neighboring menu levels.
    ((480 * dpi / 96) as i32)
        .min(work_width / 3)
        .saturating_sub((64 * dpi / 96) as i32)
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_width_scales_with_dpi_but_respects_monitor_work_area() {
        assert_eq!(label_budget(1920, 96), 416);
        assert_eq!(label_budget(3840, 192), 832);
        assert!(label_budget(800, 192) < 800 / 3);
        assert_eq!(label_budget(1, 768), 1);
    }

    #[test]
    fn native_font_measurement_keeps_captions_and_labels_within_budget() {
        let text = MenuText::new(HWND::default(), POINT::default());
        let long = format!("D:\\Documents\\{}\\Folder", "Example".repeat(100));
        let label = text.fit(&long);
        assert!(text.measure(&label) <= text.budget);
        let caption = text.caption("Window 1 — ", &long);
        assert!(text.measure(&caption) <= text.budget);
        assert!(caption.contains('…'));
    }
}
