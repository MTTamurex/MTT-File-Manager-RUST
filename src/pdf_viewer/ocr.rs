//! Windows.Media.Ocr fallback for PDF pages with no embedded text layer.
//!
//! Called by the render worker thread when Pdfium text extraction returns
//! empty (scanned / image-only PDF). Runs synchronously on the worker
//! thread (MTA) — never called from the UI thread.

use windows::{
    core::Interface,
    Graphics::Imaging::{
        BitmapAlphaMode, BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap,
    },
    Media::Ocr::OcrEngine,
    Win32::System::WinRT::IMemoryBufferByteAccess,
};

use super::renderer::PdfTextBounds;

/// A single OCR-recognised word with its bounding box in PDF point space
/// (origin bottom-left, Y up) and its text content.
pub struct OcrWord {
    pub bounds: PdfTextBounds,
    pub text: String,
}

/// Run Windows.Media.Ocr on an RGBA rendered bitmap of a PDF page.
///
/// `page_w` / `page_h` are the PDF page dimensions in points (Pdfium's
/// natural coordinate space: origin bottom-left, Y-up). The returned word
/// bounds are mapped into the same space so they are directly compatible
/// with [`PdfTextSegment`].
///
/// Returns `None` when OCR is unavailable, unsupported, or produces no
/// output. Logs a warning on unexpected failures.
pub fn ocr_page_bitmap(
    rgba_pixels: &[u8],
    bitmap_w: u32,
    bitmap_h: u32,
    page_w: f32,
    page_h: f32,
) -> Option<Vec<OcrWord>> {
    match run_ocr(rgba_pixels, bitmap_w, bitmap_h, page_w, page_h) {
        Ok(words) if !words.is_empty() => {
            log::debug!(
                "[PDF-OCR] recognised {} words on {}×{} bitmap",
                words.len(),
                bitmap_w,
                bitmap_h
            );
            Some(words)
        }
        Ok(_) => None,
        Err(e) => {
            log::warn!("[PDF-OCR] Windows OCR failed: {e}");
            None
        }
    }
}

fn run_ocr(
    rgba_pixels: &[u8],
    bitmap_w: u32,
    bitmap_h: u32,
    page_w: f32,
    page_h: f32,
) -> windows::core::Result<Vec<OcrWord>> {
    // OcrEngine has a maximum supported image dimension; skip rather than fail.
    let max_side = OcrEngine::MaxImageDimension()?;
    if bitmap_w > max_side || bitmap_h > max_side {
        log::debug!(
            "[PDF-OCR] bitmap {}×{} exceeds MaxImageDimension ({}); skipping",
            bitmap_w,
            bitmap_h,
            max_side
        );
        return Ok(vec![]);
    }

    let bitmap = create_software_bitmap(rgba_pixels, bitmap_w, bitmap_h)?;

    // Use the user's Windows display language pack.
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;

    // RecognizeAsync is awaited synchronously — safe on a worker (MTA) thread.
    let result = engine.RecognizeAsync(&bitmap)?.get()?;

    let mut words = Vec::new();
    let lines = result.Lines()?;
    for i in 0..lines.Size()? {
        let line = lines.GetAt(i)?;
        let line_words = line.Words()?;
        for j in 0..line_words.Size()? {
            let word = line_words.GetAt(j)?;
            let rect = word.BoundingRect()?;
            let text = word.Text()?.to_string();
            if text.trim().is_empty() {
                continue;
            }
            let bounds = pixel_rect_to_pdf_bounds(
                rect.X,
                rect.Y,
                rect.Width,
                rect.Height,
                bitmap_w as f32,
                bitmap_h as f32,
                page_w,
                page_h,
            );
            words.push(OcrWord { bounds, text });
        }
    }

    Ok(words)
}

/// Create a `SoftwareBitmap` (RGBA8, alpha-ignored) from raw RGBA bytes.
fn create_software_bitmap(
    rgba: &[u8],
    w: u32,
    h: u32,
) -> windows::core::Result<SoftwareBitmap> {
    let bitmap = SoftwareBitmap::CreateWithAlpha(
        BitmapPixelFormat::Rgba8,
        w as i32,
        h as i32,
        BitmapAlphaMode::Ignore,
    )?;
    {
        let buf = bitmap.LockBuffer(BitmapBufferAccessMode::Write)?;
        let reference = buf.CreateReference()?;
        // SAFETY: `IMemoryBufferByteAccess::GetBuffer` returns a valid pointer
        // and capacity that live as long as `reference` is held.
        let byte_access: IMemoryBufferByteAccess = reference.cast()?;
        unsafe {
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut capacity: u32 = 0;
            byte_access.GetBuffer(&mut ptr, &mut capacity)?;
            let len = (capacity as usize).min(rgba.len());
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), ptr, len);
        }
    }
    Ok(bitmap)
}

/// Map a pixel-space rectangle (origin top-left, Y-down) to PDF point space
/// (origin bottom-left, Y-up), matching Pdfium's coordinate convention.
fn pixel_rect_to_pdf_bounds(
    px_x: f32,
    px_y: f32,
    px_w: f32,
    px_h: f32,
    bitmap_w: f32,
    bitmap_h: f32,
    page_w: f32,
    page_h: f32,
) -> PdfTextBounds {
    let left = (px_x / bitmap_w) * page_w;
    let right = ((px_x + px_w) / bitmap_w) * page_w;
    // Flip Y: PDF origin is at the bottom.
    let top = (1.0 - px_y / bitmap_h) * page_h;
    let bottom = (1.0 - (px_y + px_h) / bitmap_h) * page_h;
    PdfTextBounds::from_points(left, right, top, bottom)
}
