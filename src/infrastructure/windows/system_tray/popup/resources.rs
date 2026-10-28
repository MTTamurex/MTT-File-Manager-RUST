//! Application palette, accessibility fallback and owned popup fonts/bitmaps.
use super::super::{badge, graphics::Bitmap, menu_icons::MenuIcons};
use std::mem::size_of;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETHIGHCONTRAST, SPI_GETNONCLIENTMETRICS,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

pub(super) struct Font(pub HFONT);
impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

pub(super) struct Resources {
    pub font: Font,
    pub bold: Font,
    pub icons: MenuIcons,
    pub app: Option<Bitmap>,
    pub icon_size: i32,
    pub colors: Colors,
}

pub(super) struct Colors {
    pub background: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub hover: COLORREF,
    pub border: COLORREF,
    pub accent: COLORREF,
    pub accent_text: COLORREF,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

impl Colors {
    fn system() -> Self {
        let mut contrast = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        let high_contrast = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .is_ok()
            && contrast.dwFlags.contains(HCF_HIGHCONTRASTON);
        if high_contrast {
            let color = |index| COLORREF(unsafe { GetSysColor(index) });
            return Self {
                background: color(COLOR_WINDOW),
                text: color(COLOR_WINDOWTEXT),
                muted: color(COLOR_WINDOWTEXT),
                hover: color(COLOR_HIGHLIGHT),
                border: color(COLOR_WINDOWTEXT),
                accent: color(COLOR_HIGHLIGHT),
                accent_text: color(COLOR_HIGHLIGHTTEXT),
            };
        }
        if let Some(ctx) = super::super::session_exit::context() {
            return Self::application(&ctx);
        }
        Self::palette(crate::infrastructure::windows::app_theme::windows_app_is_dark())
    }

    pub(super) fn application(ctx: &eframe::egui::Context) -> Self {
        let style = ctx.global_style();
        let visuals = &style.visuals;
        let background = visuals.window_fill;
        let color = |value: eframe::egui::Color32| {
            let alpha = value.a() as u32;
            let channel = |front: u8, back: u8| {
                (front as u32 + back as u32 * (255 - alpha) / 255).min(255) as u8
            };
            rgb(
                channel(value.r(), background.r()),
                channel(value.g(), background.g()),
                channel(value.b(), background.b()),
            )
        };
        Self {
            background: color(background),
            text: color(visuals.text_color()),
            muted: color(visuals.weak_text_color()),
            hover: color(visuals.widgets.hovered.bg_fill),
            border: color(visuals.window_stroke.color),
            accent: color(crate::ui::theme::accent_color(ctx)),
            accent_text: color(crate::ui::theme::accent_text_color(ctx)),
        }
    }

    pub(super) fn palette(dark: bool) -> Self {
        if dark {
            Self {
                background: rgb(28, 30, 35),
                text: rgb(245, 245, 247),
                muted: rgb(175, 181, 192),
                hover: rgb(47, 53, 64),
                border: rgb(69, 74, 85),
                accent: rgb(0, 112, 220),
                accent_text: rgb(255, 255, 255),
            }
        } else {
            Self {
                background: rgb(249, 249, 252),
                text: rgb(25, 28, 34),
                muted: rgb(89, 96, 108),
                hover: rgb(225, 234, 248),
                border: rgb(208, 213, 222),
                accent: rgb(0, 100, 205),
                accent_text: rgb(255, 255, 255),
            }
        }
    }
}

impl Resources {
    pub fn new(dpi: u32) -> Option<Self> {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some((&mut metrics as *mut NONCLIENTMETRICSW).cast()),
                0,
                dpi,
            )
        }
        .ok()?;
        let mut description = metrics.lfMenuFont;
        description.lfHeight = -(12 * dpi as i32 / 96);
        description.lfWeight = 400;
        description.lfQuality = CLEARTYPE_QUALITY;
        let font = Font(unsafe { CreateFontIndirectW(&description) });
        if font.0.is_invalid() {
            return None;
        }
        description.lfWeight = 600;
        let bold = Font(unsafe { CreateFontIndirectW(&description) });
        if bold.0.is_invalid() {
            return None;
        }
        let icon_size = (28 * dpi / 96).clamp(16, 256) as i32;
        Some(Self {
            font,
            bold,
            icons: MenuIcons::new(icon_size as u32),
            app: badge::render(1, icon_size as u32).and_then(|pixels| Bitmap::from_pixmap(&pixels)),
            icon_size,
            colors: Colors::system(),
        })
    }
}

#[cfg(test)]
mod tests;
