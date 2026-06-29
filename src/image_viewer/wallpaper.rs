//! Set desktop wallpaper via Windows SystemParametersInfoW API.

use crate::image_viewer::loader;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER,
};

/// Sets the given image as the desktop wallpaper.
///
/// Windows `SystemParametersInfoW` with `SPI_SETDESKWALLPAPER` works reliably
/// only with BMP files. For non-BMP images, this function uses the image
/// viewer decode pipeline and re-encodes as BMP to a temp file before applying.
pub fn set_as_wallpaper(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("File not found: '{}'", path.display()));
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

/// Decode the source image and save it as BMP to a temp file.
fn convert_to_temp_bmp(path: &Path) -> Result<std::path::PathBuf, String> {
    let frame =
        loader::decode_export_frame(path).map_err(|e| format!("Failed to decode image: {}", e))?;
    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .ok_or_else(|| "Decoded image buffer has invalid dimensions".to_string())?;

    let temp_dir = std::env::temp_dir();
    let bmp_path = temp_dir.join(format!("mtt_wallpaper_temp_{}.bmp", std::process::id()));

    buffer
        .save_with_format(&bmp_path, image::ImageFormat::Bmp)
        .map_err(|e| format!("Failed to save BMP: {}", e))?;

    Ok(bmp_path)
}
