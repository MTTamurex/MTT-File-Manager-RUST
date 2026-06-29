//! Set desktop wallpaper via Windows SystemParametersInfoW API.

use crate::image_viewer::loader;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
};

/// Maximum total pixel count allowed when re-encoding the source image to BMP.
/// Above this threshold the image is downscaled to keep the temp BMP and the
/// peak decode memory bounded.
const MAX_WALLPAPER_PIXELS: u64 = 16_000_000;

pub fn set_as_wallpaper_if_current<F>(path: &Path, is_current: F) -> Result<(), String>
where
    F: Fn() -> bool,
{
    if !path.is_file() {
        return Err(format!("File not found: '{}'", path.display()));
    }
    if !is_current() {
        return Err("wallpaper operation superseded".to_string());
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let bmp_path = if ext == "bmp" {
        path.to_path_buf()
    } else {
        convert_to_temp_bmp(path)?
    };

    if !is_current() {
        return Err("wallpaper operation superseded".to_string());
    }

    let wide: Vec<u16> = bmp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut _),
            SPIF_UPDATEINIFILE,
        )
    };

    result.map_err(|e| format!("SystemParametersInfoW failed: {}", e))
}

/// Decode the source image, downscale if it exceeds the wallpaper budget, and
/// save it as BMP to a per-process temp file (overwriting the previous one).
fn convert_to_temp_bmp(path: &Path) -> Result<std::path::PathBuf, String> {
    let frame =
        loader::decode_export_frame(path).map_err(|e| format!("Failed to decode image: {}", e))?;
    let mut buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .ok_or_else(|| "Decoded image buffer has invalid dimensions".to_string())?;

    let total_pixels = (buffer.width() as u64) * (buffer.height() as u64);
    if total_pixels > MAX_WALLPAPER_PIXELS {
        buffer = downscale_to_budget(buffer, MAX_WALLPAPER_PIXELS);
    }

    let temp_dir = std::env::temp_dir();
    let bmp_path = temp_dir.join(format!("mtt_wallpaper_temp_{}.bmp", std::process::id()));

    buffer
        .save_with_format(&bmp_path, image::ImageFormat::Bmp)
        .map_err(|e| format!("Failed to save BMP: {}", e))?;

    Ok(bmp_path)
}

fn downscale_to_budget(buffer: image::RgbaImage, max_pixels: u64) -> image::RgbaImage {
    let (w, h) = (buffer.width(), buffer.height());
    let current = (w as u64) * (h as u64);
    if current <= max_pixels {
        return buffer;
    }

    let scale = ((max_pixels as f64) / (current as f64)).sqrt();
    let target_w = ((w as f64) * scale).floor().max(1.0) as u32;
    let target_h = ((h as f64) * scale).floor().max(1.0) as u32;
    image::imageops::resize(
        &buffer,
        target_w,
        target_h,
        image::imageops::FilterType::Triangle,
    )
}

#[cfg(test)]
mod tests {
    use super::downscale_to_budget;

    #[test]
    fn downscale_leaves_small_image_untouched() {
        let img = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]));
        let out = downscale_to_budget(img.clone(), 16_000_000);
        assert_eq!(out.dimensions(), (64, 64));
    }

    #[test]
    fn downscale_shrinks_image_above_budget() {
        let img = image::RgbaImage::from_pixel(1_000, 1_000, image::Rgba([0, 0, 0, 255]));
        let out = downscale_to_budget(img, 250_000);
        let (w, h) = out.dimensions();
        assert!(((w as u64) * (h as u64)) <= 250_000);
        assert!(w < 1_000 && h < 1_000);
    }

    #[test]
    fn checked_wallpaper_rejects_superseded_operation_before_windows_api() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let bmp = dir.path().join("wallpaper.bmp");
        std::fs::write(&bmp, b"not a real bmp").expect("create placeholder");

        let result = super::set_as_wallpaper_if_current(&bmp, || false);
        assert_eq!(result, Err("wallpaper operation superseded".to_string()));
    }
}
