//! Stage 2: Windows Imaging Component (WIC) extraction
//!
//! Uses Windows WIC for robust fallback, especially for CMYK JPEGs
//! and other formats that the image crate may struggle with.

use std::path::Path;
use windows::{
    core::PCWSTR, Win32::Foundation::GENERIC_ACCESS_RIGHTS, Win32::Graphics::Imaging::*,
    Win32::System::Com::*,
};

/// Try to extract thumbnail using Windows Imaging Component (WIC)
///
/// Supports: jpg, jpeg, png, bmp, gif, tiff, webp, ico, tif
/// Best for: CMYK images, malformed JPEGs
pub fn extract(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // WIC is for image files only - videos should go directly to Shell API (Stage 3)
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tiff" | "webp" | "ico" | "tif"
    ) {
        return None;
    }

    unsafe {
        // SAFETY: All WIC components are used within this block and the COM library
        // has been initialized for this thread. Raw pointers from `path_wide` are
        // valid for the duration of the call.
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;

        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(path_wide.as_ptr()),
                None,
                GENERIC_ACCESS_RIGHTS(0x80000000), // GENERIC_READ
                WICDecodeMetadataCacheOnDemand,
            )
            .ok()?;

        let frame = decoder.GetFrame(0).ok()?;

        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .ok()?;

        let mut width = 0;
        let mut height = 0;
        converter.GetSize(&mut width, &mut height).ok()?;

        let mut buffer = vec![0u8; (width * height * 4) as usize];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut buffer)
            .ok()?;

        Some((buffer, width, height))
    }
}
