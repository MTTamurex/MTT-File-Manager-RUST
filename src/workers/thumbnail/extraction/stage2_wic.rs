//! Stage 2: Windows Imaging Component (WIC) extraction
//!
//! Uses Windows WIC for robust fallback, especially for CMYK JPEGs
//! and other formats that the image crate may struggle with.

use std::path::Path;
use windows::{
    core::PCWSTR, Win32::Foundation::GENERIC_ACCESS_RIGHTS, Win32::Graphics::Imaging::*,
    Win32::System::Com::*,
};

// ── Thread-local cached WIC factory ─────────────────────────────────────
//
// Creating a new IWICImagingFactory via CoCreateInstance on every decode
// is expensive: it round-trips through COM's class-factory machinery,
// and WIC's internal DLL (windowscodecs.dll) accumulates cached state
// per factory lifecycle. Over hundreds of rapid decodes this contributes
// to a slow, silent resource growth that degrades the whole OS.
//
// By caching the factory per thread we pay the creation cost only once
// per worker thread, and the factory is reused for all subsequent
// decodes on that thread.

std::thread_local! {
    static WIC_FACTORY: std::cell::RefCell<Option<IWICImagingFactory>> = const { std::cell::RefCell::new(None) };
}

/// Obtain or create the per-thread cached WIC factory.
fn get_or_create_factory() -> Option<IWICImagingFactory> {
    WIC_FACTORY.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(factory) = slot.as_ref() {
            return Some(factory.clone());
        }
        let factory: IWICImagingFactory = unsafe {
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?
        };
        *slot = Some(factory.clone());
        Some(factory)
    })
}

/// Release the per-thread cached WIC factory.
/// Called from the COM guard's Drop impl *before* CoUninitialize so that
/// the COM pointer is freed while the apartment is still valid.
///
/// Uses `try_with` to avoid panicking if this thread-local has already
/// been destroyed (TLS drop order between different thread-locals is
/// undefined on thread exit).
pub fn drop_thread_local_factory() {
    let _ = WIC_FACTORY.try_with(|cell| {
        let _ = cell.borrow_mut().take();
    });
}

/// Try to extract thumbnail using Windows Imaging Component (WIC)
///
/// Supports: jpg, jpeg, png, bmp, gif, tiff, webp, ico, tif
/// Best for: CMYK images, malformed JPEGs
pub fn extract(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    extract_to_size(path, None)
}

/// Decode an image via WIC directly to a target size.
///
/// When `max_side` is `Some(n)`, WIC decodes only the data needed to
/// produce an image whose longest side is at most `n` pixels.  This
/// avoids allocating a full-resolution buffer and is typically 5–20×
/// faster for high-megapixel JPEGs because WIC can skip high-frequency
/// DCT coefficients when subsampling.
///
/// When `max_side` is `None`, the native image resolution is returned
/// (behaviour identical to the old `extract`).
pub fn extract_to_size(path: &Path, max_side: Option<u32>) -> Option<(Vec<u8>, u32, u32)> {
    extract_to_size_with_original_size(path, max_side)
        .map(|(buffer, width, height, _, _)| (buffer, width, height))
}

/// Decode an image via WIC directly to a target size and return both final
/// buffer dimensions and native image dimensions.
pub fn extract_to_size_with_original_size(
    path: &Path,
    max_side: Option<u32>,
) -> Option<(Vec<u8>, u32, u32, u32, u32)> {
    // WIC is for image files only - videos should go directly to Shell API (Stage 3)
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tiff" | "webp" | "ico" | "tif"
    ) {
        return None;
    }

    unsafe {
        let factory = get_or_create_factory()?;

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

        // Determine native size so we can decide whether to scale.
        let mut native_width = 0u32;
        let mut native_height = 0u32;
        frame.GetSize(&mut native_width, &mut native_height).ok()?;

        let converter = factory.CreateFormatConverter().ok()?;

        if let Some(max_s) = max_side {
            let native_longest = native_width.max(native_height);
            if native_longest > max_s {
                // Compute fitted dimensions preserving aspect ratio.
                let ratio = max_s as f64 / native_longest as f64;
                let out_width = ((native_width as f64 * ratio).round() as u32).max(1);
                let out_height = ((native_height as f64 * ratio).round() as u32).max(1);

                let scaler = factory.CreateBitmapScaler().ok()?;
                scaler
                    .Initialize(
                        &frame,
                        out_width,
                        out_height,
                        WICBitmapInterpolationModeHighQualityCubic,
                    )
                    .ok()?;
                converter
                    .Initialize(
                        &scaler,
                        &GUID_WICPixelFormat32bppRGBA,
                        WICBitmapDitherTypeNone,
                        None,
                        0.0,
                        WICBitmapPaletteTypeMedianCut,
                    )
                    .ok()?;
            } else {
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
            }
        } else {
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
        }

        let mut final_width = 0u32;
        let mut final_height = 0u32;
        converter.GetSize(&mut final_width, &mut final_height).ok()?;

        let mut buffer = vec![0u8; (final_width * final_height * 4) as usize];
        converter
            .CopyPixels(std::ptr::null(), final_width * 4, &mut buffer)
            .ok()?;

        Some((buffer, final_width, final_height, native_width, native_height))
    }
}
