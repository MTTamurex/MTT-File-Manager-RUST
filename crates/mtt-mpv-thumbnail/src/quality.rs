//! Conservative near-black/white detection on at most 64 x 64 sample points.
use image::RgbaImage;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Quality {
    pub unsuitable: bool,
    pub score: f64,
    mean: f64,
    deviation: f64,
    dark_fraction: f64,
    light_fraction: f64,
}

impl Quality {
    pub fn json(self) -> Value {
        json!({"unsuitable": self.unsuitable, "score": self.score,
            "mean_luma": self.mean, "luma_deviation": self.deviation,
            "dark_fraction": self.dark_fraction, "light_fraction": self.light_fraction})
    }
}

pub(crate) fn assess(image: &RgbaImage) -> Quality {
    let mut count = 0u64;
    let mut sum = 0u64;
    let mut squares = 0u64;
    let mut dark = 0u64;
    let mut light = 0u64;
    let x_step = image.width().div_ceil(64).max(1) as usize;
    let y_step = image.height().div_ceil(64).max(1) as usize;
    for y in (0..image.height()).step_by(y_step) {
        for x in (0..image.width()).step_by(x_step) {
            let pixel = image.get_pixel(x, y).0;
            let luma =
                (54 * pixel[0] as u64 + 183 * pixel[1] as u64 + 19 * pixel[2] as u64 + 128) >> 8;
            count += 1;
            sum += luma;
            squares += luma * luma;
            dark += u64::from(luma <= 16);
            light += u64::from(luma >= 240);
        }
    }
    let divisor = count.max(1) as f64;
    let mean = sum as f64 / divisor;
    let deviation = (squares as f64 / divisor - mean * mean).max(0.0).sqrt();
    let dark_fraction = dark as f64 / divisor;
    let light_fraction = light as f64 / divisor;
    let extreme_fraction = dark_fraction.max(light_fraction);
    Quality {
        unsuitable: count == 0 || (extreme_fraction >= 0.985 && deviation <= 12.0),
        // Only ranks the bounded fallback candidates; this is not a scene classifier.
        score: deviation + 64.0 * (1.0 - extreme_fraction),
        mean,
        deviation,
        dark_fraction,
        light_fraction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn rejects_uniform_black_white_and_near_extremes() {
        for level in [0, 8, 16, 240, 248, 255] {
            assert!(
                assess(&RgbaImage::from_pixel(
                    256,
                    144,
                    Rgba([level, level, level, 255])
                ))
                .unsuitable
            );
        }
    }

    #[test]
    fn preserves_normal_color_flat_animation_and_contrast_on_bright_background() {
        for color in [[128, 128, 128, 255], [255, 0, 0, 255], [30, 30, 30, 255]] {
            assert!(!assess(&RgbaImage::from_pixel(64, 64, Rgba(color))).unsuitable);
        }
        let mut white = RgbaImage::from_pixel(64, 64, Rgba([255, 255, 255, 255]));
        for y in 20..44 {
            for x in 20..44 {
                white.put_pixel(x, y, Rgba([20, 20, 20, 255]));
            }
        }
        assert!(!assess(&white).unsuitable);
    }

    #[test]
    fn dark_scene_with_detail_is_not_a_uniform_blank() {
        let image = RgbaImage::from_fn(64, 64, |x, y| {
            let value = if (x + y) % 2 == 0 { 5 } else { 40 };
            Rgba([value, value, value, 255])
        });
        assert!(!assess(&image).unsuitable);
    }

    #[test]
    fn ranks_more_informative_fallback_above_a_uniform_frame() {
        let flat = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255]));
        let almost_flat =
            RgbaImage::from_fn(64, 64, |x, _| Rgba([u8::from(x % 2 == 0) * 8, 0, 0, 255]));
        assert!(assess(&almost_flat).score > assess(&flat).score);
        assert!(assess(&RgbaImage::new(0, 0)).unsuitable);
    }
}
