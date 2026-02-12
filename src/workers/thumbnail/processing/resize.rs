//! Image resizing utilities for thumbnail processing
//!
//! Provides bucket-based resizing to optimize GPU upload and memory usage.

use image::{DynamicImage, ImageBuffer};

/// Get the appropriate bucket size for a requested thumbnail size
pub fn get_bucket_size(req_size: u32) -> u32 {
    match req_size {
        0..=128 => 128,
        129..=256 => 256,
        257..=512 => 512,
        _ => 1024,
    }
}

/// Resize RGBA buffer to bucket size while preserving aspect ratio
pub fn resize_to_bucket(
    rgba_data: Vec<u8>,
    width: u32,
    height: u32,
    max_dim: u32,
) -> (Vec<u8>, u32, u32) {
    // Se já é pequeno o suficiente, retorna como está
    if width <= max_dim && height <= max_dim {
        return (rgba_data, width, height);
    }

    // Calcula novo tamanho mantendo aspect ratio
    let scale = (max_dim as f32) / (width.max(height) as f32);
    let new_w = ((width as f32) * scale).round() as u32;
    let new_h = ((height as f32) * scale).round() as u32;

    // Ensure we don't lose the buffer if from_raw fails (which consumes it)
    // We check the condition beforehand.
    // ImageBuffer::from_raw requires buffer.len() >= width * height * 4
    let Some(min_len) = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
    else {
        return (rgba_data, width, height); // dimensions overflow, return unchanged
    };

    if rgba_data.len() >= min_len {
        // Usa image crate para resize
        // Safe to unwrap because we checked the dimensions
        let img = ImageBuffer::from_raw(width, height, rgba_data)
            .expect("Buffer size check passed but from_raw failed");

        let dynamic = DynamicImage::ImageRgba8(img);
        // Use CatmullRom for high-quality sharpening with good performance.
        let resized = dynamic.resize(new_w, new_h, image::imageops::FilterType::CatmullRom);
        let rgba = resized.into_rgba8();
        let w = rgba.width();
        let h = rgba.height();
        return (rgba.into_vec(), w, h);
    }

    // Fallback: retorna original se resize falhar ou tamanho incorreto
    (rgba_data, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_bucket_size() {
        assert_eq!(get_bucket_size(64), 128);
        assert_eq!(get_bucket_size(128), 128);
        assert_eq!(get_bucket_size(200), 256);
        assert_eq!(get_bucket_size(512), 512);
        assert_eq!(get_bucket_size(1024), 1024);
        assert_eq!(get_bucket_size(2048), 1024);
    }

    #[test]
    fn test_resize_to_bucket_no_resize_needed() {
        let data = vec![255u8; 100 * 100 * 4]; // 100x100 RGBA
        let (result, w, h) = resize_to_bucket(data.clone(), 100, 100, 256);
        assert_eq!(w, 100);
        assert_eq!(h, 100);
        assert_eq!(result.len(), data.len());
    }

    #[test]
    fn test_resize_to_bucket_downscales() {
        let data = vec![255u8; 1024 * 1024 * 4]; // 1024x1024 RGBA
        let (_result, w, h) = resize_to_bucket(data, 1024, 1024, 256);
        // Should be resized to fit in 256 bucket while maintaining aspect ratio
        assert!(w <= 256, "Width {} should be <= 256", w);
        assert!(h <= 256, "Height {} should be <= 256", h);
        assert_eq!(w, h, "Should maintain aspect ratio (square)");
    }
}
