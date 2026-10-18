use image::RgbaImage;

/// Replace the baked-in hue with the selected palette, retaining restrained relief.
/// Alpha is never modified: color lightness must not come from transparency.
pub(super) fn recolor_shell(image: &mut RgbaImage, color: [u8; 3]) {
    for pixel in image.pixels_mut() {
        let luma = (u32::from(pixel[0]) * 54 + u32::from(pixel[1]) * 183 + u32::from(pixel[2]) * 19)
            as f32
            / 256.0;
        for (channel, target) in pixel.0[..3].iter_mut().zip(color) {
            let target = f32::from(target);
            let value = if luma <= 220.0 {
                // Compress the original shadows rather than darkening the whole asset.
                target * (0.7 + 0.3 * luma / 220.0)
            } else {
                // Lift highlights towards white without clipping the chosen hue.
                target + (255.0 - target) * (luma - 220.0) / 35.0 * 0.2
            };
            *channel = value.round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::folder_appearance::FolderAppearance;
    use image::Rgba;

    #[test]
    fn palette_reference_retains_color_and_opacity() {
        for appearance in FolderAppearance::ALL.into_iter().skip(1) {
            let color = appearance.color().unwrap();
            let mut image = RgbaImage::from_pixel(1, 1, Rgba([220, 220, 220, 127]));
            recolor_shell(&mut image, color);
            assert_eq!(image.get_pixel(0, 0).0, [color[0], color[1], color[2], 127]);
        }
    }

    #[test]
    fn pastel_yellow_is_not_the_old_darkened_mustard() {
        let mut image = RgbaImage::from_pixel(1, 1, Rgba([255, 210, 60, 255]));
        recolor_shell(&mut image, FolderAppearance::PastelYellow.color().unwrap());
        let pixel = image.get_pixel(0, 0);
        assert!(pixel[0] >= 225 && pixel[1] >= 210 && pixel[2] >= 130);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn relief_is_preserved_without_modifying_alpha() {
        let mut image =
            RgbaImage::from_fn(256, 1, |x, _| Rgba([x as u8, x as u8, x as u8, x as u8]));
        recolor_shell(&mut image, FolderAppearance::Lavender.color().unwrap());
        for x in 1..256 {
            let previous = image.get_pixel(x - 1, 0);
            let current = image.get_pixel(x, 0);
            assert_eq!(current[3], x as u8);
            for channel in 0..3 {
                assert!(current[channel] >= previous[channel]);
            }
        }
        assert_ne!(image.get_pixel(0, 0).0[..3], image.get_pixel(255, 0).0[..3]);
    }
}
