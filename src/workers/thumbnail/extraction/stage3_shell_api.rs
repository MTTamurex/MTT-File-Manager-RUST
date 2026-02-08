//! Stage 3: Windows Shell API extraction
//!
//! Uses the Windows Shell IShellItemImageFactory for universal thumbnail extraction.
//! This works for most file types including videos, documents, and executables.

use crate::infrastructure::windows::file_type::is_video_extension;
use std::path::Path;
use windows::core::Interface;
use windows::{
    core::PCWSTR,
    Win32::Graphics::Gdi::{DeleteObject, HBITMAP},
    Win32::UI::Shell::{
        IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT,
        SIIGBF_THUMBNAILONLY,
    },
};

/// Extract thumbnail using Windows Shell API
///
/// This is the universal fallback that works for most file types.
/// For videos, uses THUMBNAILONLY to fail if only an icon is available.
pub fn extract(path: &Path) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    // Determine size based on file type - use centralized extension check
    // Videos: 512px (high quality for preview panel)
    // Others: 1024px (high-res system icons, executables, etc.)
    let is_video = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| is_video_extension(&ext.to_lowercase()))
        .unwrap_or(false);

    let size_px = if is_video { 512 } else { 1024 };

    unsafe {
        // SAFETY: Raw pointers from `path_wide` are valid for the call.
        // HBITMAP is a resource that is manually deleted with `DeleteObject` below.
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;
        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        let size = windows::Win32::Foundation::SIZE {
            cx: size_px,
            cy: size_px,
        };

        // Para vídeos: usa THUMBNAILONLY para FALHAR se só tiver ícone
        // Isso permite que Stage 4 (force extraction) seja acionado
        // Para outros arquivos: usa RESIZETOFIT que aceita ícones
        let flags = if is_video {
            SIIGBF_THUMBNAILONLY
        } else {
            SIIGBF_RESIZETOFIT
        };
        let hbitmap: HBITMAP = image_factory.GetImage(size, flags)?;

        let (rgba_data, width, height) = hbitmap_to_rgba(hbitmap)?;
        let _ = DeleteObject(hbitmap.into());

        Ok((rgba_data, width, height))
    }
}

/// Convert Windows HBITMAP to RGBA byte array
fn hbitmap_to_rgba(
    hbitmap: windows::Win32::Graphics::Gdi::HBITMAP,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        // SAFETY: `bm` is properly initialized before being passed to `GetObjectW`.
        // `buffer` is pre-allocated with correct size. `hbitmap` is a valid handle.
        let mut bm = BITMAP::default();
        GetObjectW(
            hbitmap.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );

        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.unsigned_abs() as usize;
        let mut buffer = vec![0u8; width * height * 4];

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let hdc = GetDC(None);
        GetDIBits(
            hdc,
            hbitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);

        // Convert BGRA to RGBA
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        Ok((buffer, width as u32, height as u32))
    }
}
