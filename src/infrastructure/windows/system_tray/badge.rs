use std::sync::OnceLock;
use tiny_skia::{IntSize, Pixmap, PixmapPaint, Transform};

pub(super) fn label(count: usize) -> Option<&'static str> {
    match count {
        0 | 1 => None,
        2 => Some("2"),
        3 => Some("3"),
        4 => Some("4"),
        5 => Some("5"),
        6 => Some("6"),
        7 => Some("7"),
        8 => Some("8"),
        9 => Some("9"),
        _ => Some("9+"),
    }
}

pub(super) fn render(count: usize, size: u32) -> Option<Pixmap> {
    if !(16..=256).contains(&size) {
        return None;
    }
    static BASE: OnceLock<Option<image::RgbaImage>> = OnceLock::new();
    let base = BASE
        .get_or_init(|| {
            image::load_from_memory(crate::embedded_assets::APP_ICON_256_PNG)
                .ok()
                .map(|image| image.to_rgba8())
        })
        .as_ref()?;
    let resized = image::imageops::resize(base, size, size, image::imageops::FilterType::Lanczos3);
    let mut rgba = resized.into_raw();
    for pixel in rgba.chunks_exact_mut(4) {
        for channel in 0..3 {
            pixel[channel] = ((pixel[channel] as u16 * pixel[3] as u16 + 127) / 255) as u8;
        }
    }
    let mut pixels = Pixmap::from_vec(rgba, IntSize::from_wh(size, size)?)?;
    if let Some(label) = label(count) {
        let digit = match label.as_bytes()[0] {
            b'2' => "M0 1C0-.5 4-.5 4 1C4 2.4 0 3.4 0 5H4",
            b'3' => "M0 0H4L1.7 2.2C5.2 1.8 5.2 5 1.6 5H0",
            b'4' => "M3 5V0L0 3.2H4",
            b'5' => "M4 0H0V2.4H2.2C4.8 2.4 4.8 5 2.2 5H0",
            b'6' => "M3.8 0C0 0 0 3 0 3.5C0 5.6 4 5.6 4 3.5C4 1.5 0 1.5 0 3.5",
            b'7' => "M0 0H4L1 5",
            b'8' => "M2 2.4C-.7 2.4-.7 0 2 0S4.7 2.4 2 2.4C-1 2.4-1 5 2 5S5 2.4 2 2.4",
            _ => "M.2 5C4 5 4 2 4 1.5C4-.6 0-.6 0 1.5C0 3.5 4 3.5 4 1.5",
        };
        let (shape, position, scale, plus) = if label == "9+" {
            ("<rect x='.5' y='.5' width='31' height='22' rx='5'/>", "5.8 6", "2.4", "<path d='M25 8.5V15.5M21.5 12H28.5' stroke='white' stroke-width='2' stroke-linecap='round'/>")
        } else {
            (
                "<rect x='8.5' y='.5' width='23' height='22' rx='5'/>",
                "15 5.25",
                "2.5",
                "",
            )
        };
        // Vector digits avoid font lookup and remain readable at notification
        // icon sizes. No UI thread filesystem access is needed for rendering.
        let svg = format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><g fill='#1745b8' stroke='white' stroke-width='1'>{shape}</g><path d='{digit}' transform='translate({position}) scale({scale})' fill='none' stroke='white' stroke-width='1' stroke-linecap='round' stroke-linejoin='round'/>{plus}</svg>");
        let overlay = super::graphics::rasterize(&svg, size)?;
        pixels.draw_pixmap(
            0,
            0,
            overlay.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    Some(pixels)
}

#[cfg(test)]
mod tests;
