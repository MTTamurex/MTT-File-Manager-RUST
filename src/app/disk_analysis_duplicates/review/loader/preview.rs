use super::*;
use std::fs::File;

const MAX_ENCODED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;
const PREVIEW_SIDE: u32 = 224;

#[cfg(test)]
mod tests;

pub(super) fn supported(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tif" | "tiff"
        )
    })
}

pub(super) fn load(
    file: &mut File,
    size: u64,
    cancelled: &impl Fn() -> bool,
) -> (PreviewStatus, Option<PreviewImage>) {
    if size > MAX_ENCODED_BYTES {
        return (PreviewStatus::TooLarge, None);
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        if cancelled() {
            return (PreviewStatus::Unavailable, None);
        }
        // Bound bytes even if the file grows while we are reading it.
        let remaining = (MAX_ENCODED_BYTES + 1).saturating_sub(bytes.len() as u64);
        if remaining == 0 {
            return (PreviewStatus::TooLarge, None);
        }
        let count = match file.read(&mut chunk[..remaining.min(64 * 1024) as usize]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => return (PreviewStatus::Unavailable, None),
        };
        if bytes.len() as u64 + count as u64 > MAX_ENCODED_BYTES {
            return (PreviewStatus::TooLarge, None);
        }
        if bytes.try_reserve_exact(count).is_err() {
            return (PreviewStatus::Unavailable, None);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let mut reader = match image::ImageReader::new(Cursor::new(bytes)).with_guessed_format() {
        Ok(reader) => reader,
        Err(_) => return (PreviewStatus::Unavailable, None),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    if cancelled() {
        return (PreviewStatus::Unavailable, None);
    }
    let image = match reader.decode() {
        Ok(image) => image,
        Err(image::ImageError::Limits(_)) => return (PreviewStatus::TooLarge, None),
        Err(_) => return (PreviewStatus::Unavailable, None),
    };
    if cancelled() {
        return (PreviewStatus::Unavailable, None);
    }
    let rgba = image.thumbnail(PREVIEW_SIDE, PREVIEW_SIDE).to_rgba8();
    (
        PreviewStatus::Ready,
        Some(PreviewImage {
            size: [rgba.width() as usize, rgba.height() as usize],
            rgba: rgba.into_raw(),
        }),
    )
}
