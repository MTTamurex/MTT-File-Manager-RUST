//! Format conversion utilities for video frame processing
//!
//! Handles conversion from hardware-accelerated video formats to RGBA.

/// Convert NV12 format to RGBA
///
/// NV12 layout:
/// - Y plane: width*height bytes (luminance)
/// - UV plane: width*height/2 bytes (interleaved U,V pairs)
pub fn convert_nv12_to_rgba(nv12_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let y_size = width * height;

    let y_plane = &nv12_data[0..y_size];
    let uv_plane = &nv12_data[y_size..];

    let mut rgba = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let y_index = y * width + x;
            let uv_index = (y / 2) * (width / 2) * 2 + (x / 2) * 2;

            let y_val = y_plane[y_index] as i32;
            let u_val = uv_plane[uv_index] as i32 - 128;
            let v_val = uv_plane[uv_index + 1] as i32 - 128;

            // YUV to RGB conversion (BT.601 standard) optimized with integer arithmetic
            // 1.402 * 1024 = 1435.648 -> 1436
            // 0.344 * 1024 = 352.256 -> 352
            // 0.714 * 1024 = 731.136 -> 731
            // 1.772 * 1024 = 1814.528 -> 1815
            let y_shifted = y_val << 10;
            let r = ((y_shifted + 1436 * v_val) >> 10).clamp(0, 255);
            let g = ((y_shifted - 352 * u_val - 731 * v_val) >> 10).clamp(0, 255);
            let b = ((y_shifted + 1815 * u_val) >> 10).clamp(0, 255);

            let rgba_index = y_index * 4;
            rgba[rgba_index] = r as u8;
            rgba[rgba_index + 1] = g as u8;
            rgba[rgba_index + 2] = b as u8;
            rgba[rgba_index + 3] = 255; // Alpha
        }
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_nv12_to_rgba_basic() {
        // Create a simple 2x2 NV12 buffer
        // Y plane: 4 bytes (2x2)
        // UV plane: 2 bytes (2x2 subsampled to 1x2)
        let mut nv12_data = vec![0u8; 6];
        
        // Y plane: all gray (128)
        nv12_data[0..4].fill(128);
        
        // UV plane: neutral (128, 128) = no color
        nv12_data[4] = 128;
        nv12_data[5] = 128;
        
        let rgba = convert_nv12_to_rgba(&nv12_data, 2, 2);
        
        assert_eq!(rgba.len(), 16); // 2x2 RGBA = 16 bytes
        
        // With Y=128 (gray) and U=V=128 (neutral), we should get a mid-gray color
        // The exact values depend on the YUV conversion formula
        for i in 0..4 {
            let r = rgba[i * 4];
            let g = rgba[i * 4 + 1];
            let b = rgba[i * 4 + 2];
            let a = rgba[i * 4 + 3];
            
            assert_eq!(a, 255, "Alpha should be fully opaque");
            // Gray values should be approximately equal
            assert!(r.abs_diff(g) < 10, "R and G should be similar for gray");
            assert!(g.abs_diff(b) < 10, "G and B should be similar for gray");
        }
    }

    #[test]
    fn test_convert_nv12_black() {
        // Black in NV12: Y=0, U=V=128
        let mut nv12_data = vec![0u8; 6];
        // Y=0 (already zeroed)
        nv12_data[4] = 128; // U
        nv12_data[5] = 128; // V
        
        let rgba = convert_nv12_to_rgba(&nv12_data, 2, 2);
        
        // All pixels should be black (or very close)
        for i in 0..4 {
            assert!(rgba[i * 4] < 10, "Red should be near 0 for black");
            assert!(rgba[i * 4 + 1] < 10, "Green should be near 0 for black");
            assert!(rgba[i * 4 + 2] < 10, "Blue should be near 0 for black");
        }
    }

    #[test]
    fn test_convert_nv12_white() {
        // White in NV12: Y=255, U=V=128
        let mut nv12_data = vec![255u8; 6];
        nv12_data[4] = 128; // U
        nv12_data[5] = 128; // V
        
        let rgba = convert_nv12_to_rgba(&nv12_data, 2, 2);
        
        // All pixels should be white (or very close)
        for i in 0..4 {
            assert!(rgba[i * 4] > 245, "Red should be near 255 for white");
            assert!(rgba[i * 4 + 1] > 245, "Green should be near 255 for white");
            assert!(rgba[i * 4 + 2] > 245, "Blue should be near 255 for white");
        }
    }
}