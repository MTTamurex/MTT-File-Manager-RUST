use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::windows::file_flags::{
    open_sequential, open_sequential_background, open_sequential_low_priority,
};
use exif::{In, Reader as ExifReader, Tag};
use image::metadata::Orientation;
use image::{DynamicImage, ImageBuffer, Rgba};
use std::io::BufReader;
use std::path::Path;

pub fn apply_exif_orientation_to_thumbnail(
    path: &Path,
    priority: IOPriority,
    thumbnail: (Vec<u8>, u32, u32),
) -> (Vec<u8>, u32, u32) {
    let Some(orientation) = read_exif_orientation(path, priority) else {
        return thumbnail;
    };
    apply_orientation_to_rgba(thumbnail, orientation)
}

pub fn read_exif_orientation(path: &Path, priority: IOPriority) -> Option<Orientation> {
    if !may_have_exif_orientation(path) {
        return None;
    }

    let file = match priority {
        IOPriority::Interactive => open_sequential(path).ok()?,
        IOPriority::Prefetch => open_sequential_low_priority(path).ok()?,
        IOPriority::Background => open_sequential_background(path).ok()?,
    };
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let exif = ExifReader::new().read_from_container(&mut reader).ok()?;
    let raw = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|field| field.value.get_uint(0));
    Some(map_exif_orientation(raw))
}

pub fn map_exif_orientation(raw: Option<u32>) -> Orientation {
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

pub fn apply_orientation_to_rgba(
    thumbnail: (Vec<u8>, u32, u32),
    orientation: Orientation,
) -> (Vec<u8>, u32, u32) {
    if orientation == Orientation::NoTransforms {
        return thumbnail;
    }

    let (rgba, width, height) = thumbnail;
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if expected_len != Some(rgba.len()) {
        return (rgba, width, height);
    }

    let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, rgba)
        .expect("validated RGBA dimensions should create an image buffer");
    let rotated = apply_orientation(DynamicImage::ImageRgba8(buffer), orientation).to_rgba8();
    let width = rotated.width();
    let height = rotated.height();
    (rotated.into_raw(), width, height)
}

pub fn apply_orientation(img: DynamicImage, orientation: Orientation) -> DynamicImage {
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

fn may_have_exif_orientation(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ext.eq_ignore_ascii_case("jpg")
                || ext.eq_ignore_ascii_case("jpeg")
                || ext.eq_ignore_ascii_case("tif")
                || ext.eq_ignore_ascii_case("tiff")
        })
}

#[cfg(test)]
mod tests {
    use super::{apply_orientation_to_rgba, map_exif_orientation};
    use image::metadata::Orientation;

    #[test]
    fn map_exif_orientation_matches_expected_transforms() {
        assert_eq!(map_exif_orientation(Some(1)), Orientation::NoTransforms);
        assert_eq!(map_exif_orientation(Some(6)), Orientation::Rotate90);
        assert_eq!(map_exif_orientation(Some(8)), Orientation::Rotate270);
        assert_eq!(map_exif_orientation(Some(999)), Orientation::NoTransforms);
        assert_eq!(map_exif_orientation(None), Orientation::NoTransforms);
    }

    #[test]
    fn apply_orientation_to_rgba_swaps_dimensions_for_portrait_rotation() {
        let rgba = vec![
            255, 0, 0, 255, // red
            0, 255, 0, 255, // green
        ];
        let (rotated, width, height) =
            apply_orientation_to_rgba((rgba, 2, 1), Orientation::Rotate90);

        assert_eq!((width, height), (1, 2));
        assert_eq!(rotated.len(), 2 * 4);
    }
}
