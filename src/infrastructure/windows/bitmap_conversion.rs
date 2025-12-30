//! Bitmap and icon conversion functions
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::infrastructure::windows_api::{
    Win32::Graphics::Gdi::*,
    Win32::UI::WindowsAndMessaging::*,
};

/// Converts HBITMAP to RGBA buffer.
///
/// # Safety
/// Uses GetObjectW, GetDIBits. Does NOT delete the HBITMAP (caller's responsibility).
pub fn hbitmap_to_rgba(hbitmap: HBITMAP) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(
            hbitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        
        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.abs() as usize;
        
        let mut buffer = vec![0u8; width * height * 4];
        
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
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
        
        // BGRA → RGBA conversion
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

/// Converts HICON to RGBA buffer.
///
/// Similar to hbitmap_to_rgba but works with icons (which have masks).
///
/// # Safety
/// Uses GetIconInfo, GetDIBits. Does NOT free the HICON (caller's responsibility).
/// Windows GDI returns Pre-Multiplied Alpha.
pub fn hicon_to_rgba(hicon: HICON) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let mut icon_info = ICONINFO::default();
        if GetIconInfo(hicon, &mut icon_info).is_err() {
            return Err("GetIconInfo failed".into());
        }
        
        let hbm_color = icon_info.hbmColor;
        
        let mut bm = BITMAP::default();
        GetObjectW(
            hbm_color,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        
        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.abs() as usize;
        
        // Validate size (icons are usually small, but be defensive)
        if width > 256 || height > 256 {
            let _ = DeleteObject(hbm_color);
            let _ = DeleteObject(icon_info.hbmMask);
            return Err("Icon too large".into());
        }
        
        let mut buffer = vec![0u8; width * height * 4];
        
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),  // Top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        
        let hdc = GetDC(None);
        let result = GetDIBits(
            hdc,
            hbm_color,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        
        ReleaseDC(None, hdc);
        
        if result == 0 {
            let _ = DeleteObject(hbm_color);
            let _ = DeleteObject(icon_info.hbmMask);
            return Err("GetDIBits failed".into());
        }
        
        // Cleanup bitmaps (but NOT the HICON - caller is responsible)
        let _ = DeleteObject(hbm_color);
        let _ = DeleteObject(icon_info.hbmMask);
        
        // BGRA → RGBA conversion
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

/// Creates a gray gradient placeholder for failed thumbnail extraction.
pub fn create_error_placeholder() -> (Vec<u8>, u32, u32) {
    let size = 256;
    let mut buffer = vec![0u8; size * size * 4];
    
    for (i, pixel) in buffer.chunks_exact_mut(4).enumerate() {
        let x = i % size;
        let y = i / size;
        let intensity = ((x + y) as f32 / (size * 2) as f32 * 100.0) as u8 + 100;
        pixel[0] = intensity;
        pixel[1] = intensity;
        pixel[2] = intensity;
        pixel[3] = 255;
    }
    
    (buffer, 256, 256)
}
