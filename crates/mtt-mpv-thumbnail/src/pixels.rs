use crate::ffi::Node;
use anyhow::{bail, ensure, Result};
use image::RgbaImage;

pub(crate) struct Screenshot {
    pub image: RgbaImage,
    pub width: u32,
    pub height: u32,
}

pub fn screenshot(node: &Node, max_side: u32) -> Result<Screenshot> {
    let width = u32::try_from(node.field("w")?.integer()?)?;
    let height = u32::try_from(node.field("h")?.integer()?)?;
    let stride = usize::try_from(node.field("stride")?.integer()?)?;
    let format = node.field("format")?.string()?;
    let data = node.field("data")?.bytes()?;
    let rgba = unpack(width, height, stride, format, data)?;
    let image = image::DynamicImage::ImageRgba8(rgba)
        .thumbnail(max_side, max_side)
        .into_rgba8();
    Ok(Screenshot {
        image,
        width,
        height,
    })
}

fn unpack(width: u32, height: u32, stride: usize, format: &str, data: &[u8]) -> Result<RgbaImage> {
    ensure!(width > 0 && height > 0, "Empty screenshot");
    let row = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| anyhow::anyhow!("Row overflow"))?;
    let length = row
        .checked_mul(height as usize)
        .ok_or_else(|| anyhow::anyhow!("Image overflow"))?;
    ensure!(
        length <= 256 * 1024 * 1024,
        "Screenshot exceeds the 256 MiB POC copy limit"
    );
    ensure!(stride >= row, "Screenshot stride is shorter than a row");
    let required = stride
        .checked_mul(height as usize - 1)
        .and_then(|n| n.checked_add(row))
        .ok_or_else(|| anyhow::anyhow!("Stride overflow"))?;
    ensure!(data.len() >= required, "Truncated screenshot buffer");
    ensure!(
        matches!(format, "bgr0" | "bgra" | "rgba"),
        "Unsupported screenshot format: {format}"
    );
    let mut pixels = Vec::with_capacity(length);
    for y in 0..height as usize {
        for pixel in data[y * stride..y * stride + row].as_chunks::<4>().0 {
            match format {
                "bgr0" => pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]),
                "bgra" => pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]),
                "rgba" => pixels.extend_from_slice(pixel),
                _ => bail!("Unsupported pixel format"),
            }
        }
    }
    RgbaImage::from_raw(width, height, pixels)
        .ok_or_else(|| anyhow::anyhow!("Invalid RGBA dimensions"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgr_padding_and_opaque_alpha() {
        let image = unpack(1, 2, 8, "bgr0", &[1, 2, 3, 0, 99, 99, 99, 99, 4, 5, 6, 0]).unwrap();
        assert_eq!(image.into_raw(), [3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn alpha_and_channel_order() {
        assert_eq!(
            unpack(1, 1, 4, "bgra", &[1, 2, 3, 4]).unwrap().into_raw(),
            [3, 2, 1, 4]
        );
        assert_eq!(
            unpack(1, 1, 4, "rgba", &[1, 2, 3, 4]).unwrap().into_raw(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn rejects_invalid_layouts() {
        assert!(unpack(0, 1, 4, "bgr0", &[]).is_err());
        assert!(unpack(2, 1, 4, "bgr0", &[0; 8]).is_err());
        assert!(unpack(1, 2, 8, "bgr0", &[0; 11]).is_err());
        assert!(unpack(1, 1, 4, "rgb24", &[0; 4]).is_err());
        assert!(unpack(u32::MAX, u32::MAX, usize::MAX, "rgba", &[]).is_err());
    }
}
