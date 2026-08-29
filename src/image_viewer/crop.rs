use crate::image_viewer::loader::{self, DecodedFrame};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedCrop {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

impl NormalizedCrop {
    pub fn new(start: [f32; 2], end: [f32; 2]) -> Option<Self> {
        let min_x = start[0].min(end[0]).clamp(0.0, 1.0);
        let min_y = start[1].min(end[1]).clamp(0.0, 1.0);
        let max_x = start[0].max(end[0]).clamp(0.0, 1.0);
        let max_y = start[1].max(end[1]).clamp(0.0, 1.0);
        if max_x - min_x <= f32::EPSILON || max_y - min_y <= f32::EPSILON {
            return None;
        }

        Some(Self {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }
}

pub fn crop_displayed_frame(
    frame: DecodedFrame,
    crop: NormalizedCrop,
    rotation: u16,
) -> Result<DecodedFrame, String> {
    let rotation = rotation % 360;
    if !matches!(rotation, 0 | 90 | 180 | 270) {
        return Err(format!("Unsupported crop rotation: {rotation}"));
    }

    let (display_width, display_height) = if matches!(rotation, 90 | 270) {
        (frame.height, frame.width)
    } else {
        (frame.width, frame.height)
    };
    let (display_x0, display_x1) = pixel_bounds(crop.min_x, crop.max_x, display_width)?;
    let (display_y0, display_y1) = pixel_bounds(crop.min_y, crop.max_y, display_height)?;

    let (source_x0, source_y0, source_x1, source_y1) = match rotation {
        90 => (
            display_y0,
            frame.height - display_x1,
            display_y1,
            frame.height - display_x0,
        ),
        180 => (
            frame.width - display_x1,
            frame.height - display_y1,
            frame.width - display_x0,
            frame.height - display_y0,
        ),
        270 => (
            frame.width - display_y1,
            display_x0,
            frame.width - display_y0,
            display_x1,
        ),
        _ => (display_x0, display_y0, display_x1, display_y1),
    };

    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
        .ok_or_else(|| "RGBA buffer size does not match image dimensions".to_string())?;
    let cropped = image::imageops::crop_imm(
        &buffer,
        source_x0,
        source_y0,
        source_x1 - source_x0,
        source_y1 - source_y0,
    )
    .to_image();
    let width = cropped.width();
    let height = cropped.height();
    let cropped_frame = DecodedFrame {
        rgba: cropped.into_raw(),
        width,
        height,
        original_width: width,
        original_height: height,
    };

    loader::rotate_frame(cropped_frame, rotation)
}

fn pixel_bounds(min: f32, max: f32, size: u32) -> Result<(u32, u32), String> {
    if size == 0 {
        return Err("Cannot crop an empty image".to_string());
    }

    let start = (min.clamp(0.0, 1.0) * size as f32).floor() as u32;
    let end = (max.clamp(0.0, 1.0) * size as f32).ceil() as u32;
    let start = start.min(size - 1);
    let end = end.clamp(start + 1, size);
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_frame(width: u32, height: u32) -> DecodedFrame {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for value in 1..=width * height {
            rgba.extend_from_slice(&[value as u8, 0, 0, 255]);
        }
        DecodedFrame {
            rgba,
            width,
            height,
            original_width: width,
            original_height: height,
        }
    }

    fn red_values(frame: &DecodedFrame) -> Vec<u8> {
        frame.rgba.chunks_exact(4).map(|pixel| pixel[0]).collect()
    }

    #[test]
    fn crop_uses_full_image_pixel_bounds() {
        let crop = NormalizedCrop::new([0.25, 0.0], [0.75, 1.0]).unwrap();
        let result = crop_displayed_frame(test_frame(4, 2), crop, 0).unwrap();

        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(red_values(&result), vec![2, 3, 6, 7]);
    }

    #[test]
    fn crop_matches_clockwise_display_rotation() {
        let crop = NormalizedCrop::new([0.0, 0.0], [0.5, 1.0]).unwrap();
        let result = crop_displayed_frame(test_frame(3, 2), crop, 90).unwrap();

        assert_eq!((result.width, result.height), (1, 3));
        assert_eq!(red_values(&result), vec![4, 5, 6]);
    }

    #[test]
    fn crop_matches_counterclockwise_display_rotation() {
        let crop = NormalizedCrop::new([0.0, 0.0], [0.5, 1.0]).unwrap();
        let result = crop_displayed_frame(test_frame(3, 2), crop, 270).unwrap();

        assert_eq!((result.width, result.height), (1, 3));
        assert_eq!(red_values(&result), vec![3, 2, 1]);
    }

    #[test]
    fn crop_matches_upside_down_display_rotation() {
        let crop = NormalizedCrop::new([0.0, 0.0], [1.0 / 3.0, 1.0]).unwrap();
        let result = crop_displayed_frame(test_frame(3, 2), crop, 180).unwrap();

        assert_eq!((result.width, result.height), (1, 2));
        assert_eq!(red_values(&result), vec![6, 3]);
    }

    #[test]
    fn reversed_selection_is_normalized() {
        let crop = NormalizedCrop::new([0.75, 1.0], [0.25, 0.0]).unwrap();
        assert_eq!(crop, NormalizedCrop::new([0.25, 0.0], [0.75, 1.0]).unwrap());
    }
}
