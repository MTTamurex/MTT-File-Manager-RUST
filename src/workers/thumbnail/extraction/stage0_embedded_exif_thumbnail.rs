//! Stage EXIF: Embedded JPEG thumbnail extraction
//!
//! Many camera/phone JPEGs embed a small preview JPEG in EXIF IFD1. When that
//! preview is large enough for the requested thumbnail bucket, decoding it is
//! dramatically cheaper than touching the full-resolution image on HDDs.

use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::windows::file_flags::{
    open_sequential, open_sequential_background, open_sequential_low_priority,
};
use exif::{In, Reader as ExifReader, Tag};
use image::metadata::Orientation;
use image::{DynamicImage, ImageFormat};
use std::io::BufReader;
use std::path::Path;

pub fn extract(
    path: &Path,
    priority: IOPriority,
    min_longest_side: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    if !is_jpeg_path(path) {
        return None;
    }

    let file = match priority {
        IOPriority::Interactive => open_sequential(path).ok()?,
        IOPriority::Prefetch => open_sequential_low_priority(path).ok()?,
        IOPriority::Background => open_sequential_background(path).ok()?,
    };

    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let exif = ExifReader::new().read_from_container(&mut reader).ok()?;

    let thumb_offset = exif
        .get_field(Tag::JPEGInterchangeFormat, In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let thumb_len = exif
        .get_field(Tag::JPEGInterchangeFormatLength, In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let thumb_bytes = slice_embedded_thumbnail(exif.buf(), thumb_offset, thumb_len)?;

    if !looks_like_jpeg(thumb_bytes) {
        return None;
    }

    let orientation = map_exif_orientation(
        exif.get_field(Tag::Orientation, In::PRIMARY)
            .and_then(|field| field.value.get_uint(0)),
    );

    let image = image::load_from_memory_with_format(thumb_bytes, ImageFormat::Jpeg).ok()?;
    let image = apply_orientation(image, orientation);
    let rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();

    if width.max(height) < min_longest_side.max(1) {
        return None;
    }

    Some((rgba.into_vec(), width, height))
}

fn is_jpeg_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg"))
}

fn looks_like_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0] == 0xFF && bytes[1] == 0xD8
}

fn slice_embedded_thumbnail(buf: &[u8], offset: usize, len: usize) -> Option<&[u8]> {
    let end = offset.checked_add(len)?;
    if len == 0 {
        return None;
    }
    buf.get(offset..end)
}

fn map_exif_orientation(raw: Option<u32>) -> Orientation {
    match raw {
        Some(2) => Orientation::FlipHorizontal,
        Some(3) => Orientation::Rotate180,
        Some(4) => Orientation::FlipVertical,
        Some(5) => Orientation::Rotate90FlipH,
        Some(6) => Orientation::Rotate90,
        Some(7) => Orientation::Rotate270FlipH,
        Some(8) => Orientation::Rotate270,
        _ => Orientation::NoTransforms,
    }
}

fn apply_orientation(img: DynamicImage, orientation: Orientation) -> DynamicImage {
    match orientation {
        Orientation::NoTransforms => img,
        Orientation::FlipHorizontal => img.fliph(),
        Orientation::Rotate180 => img.rotate180(),
        Orientation::FlipVertical => img.flipv(),
        Orientation::Rotate90 => img.rotate90(),
        Orientation::Rotate90FlipH => img.rotate90().fliph(),
        Orientation::Rotate270 => img.rotate270(),
        Orientation::Rotate270FlipH => img.rotate270().fliph(),
    }
}

#[cfg(test)]
mod tests {
    use super::{map_exif_orientation, slice_embedded_thumbnail};
    use image::metadata::Orientation;

    #[test]
    fn slice_embedded_thumbnail_rejects_invalid_ranges() {
        let buf = [1u8, 2, 3, 4];
        assert_eq!(slice_embedded_thumbnail(&buf, 0, 0), None);
        assert_eq!(slice_embedded_thumbnail(&buf, 4, 1), None);
        assert_eq!(slice_embedded_thumbnail(&buf, usize::MAX, 1), None);
    }

    #[test]
    fn map_exif_orientation_matches_expected_transforms() {
        assert_eq!(map_exif_orientation(Some(1)), Orientation::NoTransforms);
        assert_eq!(map_exif_orientation(Some(6)), Orientation::Rotate90);
        assert_eq!(map_exif_orientation(Some(8)), Orientation::Rotate270);
        assert_eq!(map_exif_orientation(Some(999)), Orientation::NoTransforms);
        assert_eq!(map_exif_orientation(None), Orientation::NoTransforms);
    }
}
