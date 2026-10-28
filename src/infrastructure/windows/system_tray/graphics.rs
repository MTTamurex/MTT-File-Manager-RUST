//! Owned Win32 graphics handles, converted from premultiplied RGBA pixmaps.
use std::{ffi::c_void, mem::size_of};
use tiny_skia::Pixmap;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

pub(super) struct Bitmap(pub HBITMAP);

impl Bitmap {
    pub(super) fn from_pixmap(pixels: &Pixmap) -> Option<Self> {
        Self::from_pixmap_with_alpha(pixels, false)
    }

    fn from_pixmap_with_alpha(pixels: &Pixmap, straight_alpha: bool) -> Option<Self> {
        let (width, height) = (pixels.width(), pixels.height());
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = Self(unsafe {
            CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?
        });
        if bitmap.0.is_invalid() || bits.is_null() {
            return None;
        }
        let destination =
            unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), pixels.data().len()) };
        for (rgba, bgra) in pixels
            .data()
            .chunks_exact(4)
            .zip(destination.chunks_exact_mut(4))
        {
            let channel = |value: u8| {
                if straight_alpha && rgba[3] != 0 {
                    ((value as u32 * 255 + rgba[3] as u32 / 2) / rgba[3] as u32).min(255) as u8
                } else {
                    value
                }
            };
            bgra.copy_from_slice(&[
                channel(rgba[2]),
                channel(rgba[1]),
                channel(rgba[0]),
                rgba[3],
            ]);
        }
        Some(bitmap)
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

pub(super) struct Icon(pub HICON);

impl Icon {
    pub(super) fn from_pixmap(pixels: &Pixmap) -> Option<Self> {
        // Unlike native menu bitmaps, CreateIconIndirect takes straight RGB:
        // Windows applies alpha when drawing the resulting icon.
        let color = Bitmap::from_pixmap_with_alpha(pixels, true)?;
        // Monochrome scanlines are WORD-aligned. Transparent pixels also need
        // an AND mask for consumers that do not use the 32-bit alpha channel.
        let stride = pixels.width().div_ceil(16) as usize * 2;
        let mut mask_bits = vec![0u8; stride * pixels.height() as usize];
        for (index, pixel) in pixels.data().chunks_exact(4).enumerate() {
            if pixel[3] == 0 {
                let x = index % pixels.width() as usize;
                let y = index / pixels.width() as usize;
                mask_bits[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
        let mask = Bitmap(unsafe {
            CreateBitmap(
                pixels.width() as i32,
                pixels.height() as i32,
                1,
                1,
                Some(mask_bits.as_ptr().cast()),
            )
        });
        if mask.0.is_invalid() {
            return None;
        }
        let info = ICONINFO {
            fIcon: true.into(),
            hbmColor: color.0,
            hbmMask: mask.0,
            ..Default::default()
        };
        // CreateIconIndirect copies the bitmaps, which are released on return.
        unsafe { CreateIconIndirect(&info).ok().map(Self) }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}

pub(super) fn rasterize(svg: &str, size: u32) -> Option<Pixmap> {
    if !(1..=256).contains(&size) {
        return None;
    }
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let mut pixels = Pixmap::new(size, size)?;
    let transform = tiny_skia::Transform::from_scale(
        size as f32 / tree.size().width(),
        size as f32 / tree.size().height(),
    );
    resvg::render(&tree, transform, &mut pixels.as_mut());
    Some(pixels)
}

#[cfg(test)]
mod tests;
