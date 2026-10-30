use super::*;
use std::io::{Seek, Write};

fn encoded(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([30, 60, 90, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn preview(bytes: &[u8]) -> (PreviewStatus, Option<PreviewImage>) {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(bytes).unwrap();
    file.rewind().unwrap();
    load(&mut file, bytes.len() as u64, &|| false)
}

#[test]
fn real_png_is_bounded_and_preserves_pixels() {
    let (status, image) = preview(&encoded(448, 224));
    assert_eq!(status, PreviewStatus::Ready);
    let image = image.unwrap();
    assert_eq!(image.size, [224, 112]);
    assert_eq!(image.rgba.len(), 224 * 112 * 4);
    assert_eq!(&image.rgba[..4], &[30, 60, 90, 255]);
}

#[test]
fn corrupt_and_oversized_images_are_rejected() {
    assert_eq!(preview(b"not an image").0, PreviewStatus::Unavailable);
    assert_eq!(
        preview(&encoded(MAX_DIMENSION + 1, 1)).0,
        PreviewStatus::TooLarge
    );
    let mut file = tempfile::tempfile().unwrap();
    file.set_len(MAX_ENCODED_BYTES + 1).unwrap();
    assert_eq!(
        load(&mut file, MAX_ENCODED_BYTES + 1, &|| false).0,
        PreviewStatus::TooLarge
    );
    assert_eq!(file.stream_position().unwrap(), 0);
    // Even a file which grew after metadata capture cannot exceed the budget.
    assert_eq!(load(&mut file, 0, &|| false).0, PreviewStatus::TooLarge);
}

#[test]
fn cancellation_does_not_read_encoded_content() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&encoded(2, 2)).unwrap();
    file.rewind().unwrap();
    assert_eq!(load(&mut file, 0, &|| true).0, PreviewStatus::Unavailable);
    assert_eq!(file.stream_position().unwrap(), 0);
}
