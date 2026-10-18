//! Custom folder cover composition
//!
//! Composes a folder preview by overlaying three layers:
//! 1. `folder_back_512.png`  - folder silhouette background
//! 2. Media thumbnail        - content preview (image/video from inside the folder)
//! 3. `folder_front_512.png` - folder front flap overlay
//!
//! Both PNGs are 512px wide with transparent backgrounds. When overlaid at the
//! same width and bottom-aligned, the folder pieces align perfectly (as in Photoshop).
//!
//! This replaces the Windows Shell API folder preview ("sandwich" effect)
//! to avoid issues with black backgrounds, missing thumbnails, and icon fallbacks
//! that the OS-generated previews frequently exhibit.
//!
//! PERFORMANCE:
//! - PNG layers are decoded ONCE at startup (~2ms) and kept in memory
//! - Each composition takes ~1-2ms (resize + two alpha-blend passes)
//! - Much faster than Shell API (20-200ms per folder via COM interop)

use image::{imageops, DynamicImage, RgbaImage};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

mod appearance;

/// Default output width used for the static folder icon.
pub const DEFAULT_OUTPUT_W: u32 = 256;

const BUCKET_OUTPUT_WIDTHS: [u32; 4] = [128, 256, 512, 1024];

/// Content thumbnail area at DEFAULT_OUTPUT_W scale.
const CONTENT_MARGIN_LEFT: u32 = 10;
const CONTENT_MARGIN_RIGHT: u32 = 10;
const CONTENT_MARGIN_TOP: u32 = 30;
const CONTENT_MARGIN_BOTTOM: u32 = 0;

struct FolderCompositionLayers {
    output_w: u32,
    back: RgbaImage,
    front: RgbaImage,
    paper_sheet: RgbaImage,
    canvas_h: u32,
    back_y: u32,
    front_y: u32,
    content_margin_left: u32,
    content_margin_right: u32,
    content_margin_top: u32,
    content_margin_bottom: u32,
}

/// Lazily decoded and scaled folder composition layers.
///
/// The embedded PNGs are decoded once at startup. Bucket-specific scaled layers
/// are built on first use and then shared by all folder preview workers.
pub struct FolderComposer {
    back_img: DynamicImage,
    front_img: DynamicImage,
    sheet_img: DynamicImage,
    layers: Mutex<HashMap<u32, Arc<FolderCompositionLayers>>>,
}

impl FolderComposer {
    /// Changes only the folder shell; paper and media retain their original colors.
    pub fn with_appearance(
        mut self,
        appearance: crate::domain::folder_appearance::FolderAppearance,
    ) -> Self {
        if let Some(color) = appearance.color() {
            for image in [&mut self.back_img, &mut self.front_img] {
                let mut rgba = image.to_rgba8();
                appearance::recolor_shell(&mut rgba, color);
                *image = DynamicImage::ImageRgba8(rgba);
            }
            self.layers.get_mut().clear();
        }
        self
    }

    /// Decodes the embedded folder PNG layers. Bucket scaling happens on demand.
    ///
    /// Called ONCE at app startup. Returns `None` if any embedded PNG fails to
    /// decode (should never happen since they're compile-time embedded, but the
    /// bootstrap treats folder preview as a degradable component instead of
    /// panicking the whole application).
    pub fn try_new() -> Option<Self> {
        let back_img = image::load(
            Cursor::new(crate::embedded_assets::FOLDER_BACK_PNG),
            image::ImageFormat::Png,
        )
        .map_err(|e| log::error!("[FOLDER COMPOSE] Failed to decode folder_back.png: {:?}", e))
        .ok()?;

        let front_img = image::load(
            Cursor::new(crate::embedded_assets::FOLDER_FRONT_PNG),
            image::ImageFormat::Png,
        )
        .map_err(|e| {
            log::error!(
                "[FOLDER COMPOSE] Failed to decode folder_front.png: {:?}",
                e
            )
        })
        .ok()?;

        let sheet_img = image::load(
            Cursor::new(crate::embedded_assets::PAPER_SHEET_PNG),
            image::ImageFormat::Png,
        )
        .map_err(|e| log::error!("[FOLDER COMPOSE] Failed to decode paper_sheet.png: {:?}", e))
        .ok()?;

        Some(Self {
            back_img,
            front_img,
            sheet_img,
            layers: Mutex::new(HashMap::with_capacity(BUCKET_OUTPUT_WIDTHS.len())),
        })
    }

    fn build_layers(
        output_w: u32,
        back_img: &DynamicImage,
        front_img: &DynamicImage,
        sheet_img: &DynamicImage,
    ) -> FolderCompositionLayers {
        let back = back_img
            .resize(output_w, u32::MAX, imageops::FilterType::CatmullRom)
            .to_rgba8();
        let front = front_img
            .resize(output_w, u32::MAX, imageops::FilterType::CatmullRom)
            .to_rgba8();

        let canvas_h = back.height().max(front.height());
        let back_y = canvas_h.saturating_sub(back.height());
        let front_y = canvas_h.saturating_sub(front.height());

        let content_margin_left = scale_margin(CONTENT_MARGIN_LEFT, output_w);
        let content_margin_right = scale_margin(CONTENT_MARGIN_RIGHT, output_w);
        let content_margin_top = scale_margin(CONTENT_MARGIN_TOP, output_w);
        let content_margin_bottom = scale_margin(CONTENT_MARGIN_BOTTOM, output_w);

        let gap_w = output_w.saturating_sub(content_margin_left + content_margin_right);
        let paper_sheet = sheet_img
            .resize(gap_w, u32::MAX, imageops::FilterType::CatmullRom)
            .to_rgba8();

        FolderCompositionLayers {
            output_w,
            back,
            front,
            paper_sheet,
            canvas_h,
            back_y,
            front_y,
            content_margin_left,
            content_margin_right,
            content_margin_top,
            content_margin_bottom,
        }
    }

    fn layers_for(&self, output_w: u32) -> Arc<FolderCompositionLayers> {
        let output_w = if BUCKET_OUTPUT_WIDTHS.contains(&output_w) {
            output_w
        } else {
            DEFAULT_OUTPUT_W
        };

        let mut cache = self.layers.lock();
        cache
            .entry(output_w)
            .or_insert_with(|| {
                log::info!("[FOLDER COMPOSE] Built layers for bucket {}", output_w);
                Arc::new(Self::build_layers(
                    output_w,
                    &self.back_img,
                    &self.front_img,
                    &self.sheet_img,
                ))
            })
            .clone()
    }

    /// Composes a default-size folder cover with no media content.
    pub fn compose_empty(&self) -> (Vec<u8>, u32, u32) {
        self.compose_empty_for_size(DEFAULT_OUTPUT_W)
    }

    /// Composes a bucket-sized folder cover with no media content.
    pub fn compose_empty_for_size(&self, output_w: u32) -> (Vec<u8>, u32, u32) {
        let layers = self.layers_for(output_w);
        let sheet = &layers.paper_sheet;
        let result = self.compose_with_layers(
            layers.as_ref(),
            sheet.as_raw(),
            sheet.width(),
            sheet.height(),
        );
        result.unwrap_or_else(|| {
            let mut canvas = RgbaImage::new(layers.output_w, layers.canvas_h);
            let bx = layers.output_w.saturating_sub(layers.back.width()) / 2;
            imageops::overlay(&mut canvas, &layers.back, bx as i64, layers.back_y as i64);
            let fx = layers.output_w.saturating_sub(layers.front.width()) / 2;
            imageops::overlay(&mut canvas, &layers.front, fx as i64, layers.front_y as i64);
            let w = canvas.width();
            let h = canvas.height();
            (canvas.into_raw(), w, h)
        })
    }

    /// Composes a default-size folder cover image from a media thumbnail.
    pub fn compose(
        &self,
        content_rgba: &[u8],
        content_w: u32,
        content_h: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        self.compose_for_size(content_rgba, content_w, content_h, DEFAULT_OUTPUT_W)
    }

    /// Composes a bucket-sized folder cover image from a media thumbnail.
    pub fn compose_for_size(
        &self,
        content_rgba: &[u8],
        content_w: u32,
        content_h: u32,
        output_w: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let layers = self.layers_for(output_w);
        self.compose_with_layers(layers.as_ref(), content_rgba, content_w, content_h)
    }

    fn compose_with_layers(
        &self,
        layers: &FolderCompositionLayers,
        content_rgba: &[u8],
        content_w: u32,
        content_h: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let expected = (content_w as usize)
            .checked_mul(content_h as usize)?
            .checked_mul(4)?;
        if content_rgba.len() != expected || content_w == 0 || content_h == 0 {
            return None;
        }

        let mut canvas = RgbaImage::new(layers.output_w, layers.canvas_h);

        let bx = layers.output_w.saturating_sub(layers.back.width()) / 2;
        imageops::overlay(&mut canvas, &layers.back, bx as i64, layers.back_y as i64);

        let gap_top = layers.back_y + layers.content_margin_top;
        let gap_bottom = layers.front_y.saturating_sub(layers.content_margin_bottom);
        let gap_h = gap_bottom.saturating_sub(gap_top);
        let gap_w = layers
            .output_w
            .saturating_sub(layers.content_margin_left + layers.content_margin_right);

        if gap_h > 0 && gap_w > 0 {
            let content_img = RgbaImage::from_raw(content_w, content_h, content_rgba.to_vec())?;
            let content_dyn = DynamicImage::ImageRgba8(content_img);

            let scale = gap_w as f32 / content_w as f32;
            let scaled_h = (content_h as f32 * scale).round() as u32;
            let scaled = content_dyn
                .resize_exact(gap_w, scaled_h, imageops::FilterType::CatmullRom)
                .to_rgba8();

            let max_h = layers.canvas_h.saturating_sub(gap_top);
            let crop_h = scaled_h.min(max_h);
            let cropped = imageops::crop_imm(&scaled, 0, 0, gap_w, crop_h).to_image();

            let cx = layers.content_margin_left + gap_w.saturating_sub(cropped.width()) / 2;
            let cy = gap_top;
            imageops::overlay(&mut canvas, &cropped, cx as i64, cy as i64);
        }

        let fx = layers.output_w.saturating_sub(layers.front.width()) / 2;
        imageops::overlay(&mut canvas, &layers.front, fx as i64, layers.front_y as i64);

        let w = canvas.width();
        let h = canvas.height();
        Some((canvas.into_raw(), w, h))
    }
}

fn scale_margin(value: u32, output_w: u32) -> u32 {
    ((value as u64 * output_w as u64 + (DEFAULT_OUTPUT_W / 2) as u64) / DEFAULT_OUTPUT_W as u64)
        as u32
}

/// A valid, fully transparent placeholder folder icon.
///
/// Used as the default folder icon when [`FolderComposer::try_new`] fails, so
/// the bootstrap can keep starting without a real composed icon. The buffer
/// satisfies the `width * height * 4 == len` RGBA invariant.
pub fn placeholder_folder_icon() -> (Vec<u8>, u32, u32) {
    let w = DEFAULT_OUTPUT_W;
    let h = DEFAULT_OUTPUT_W;
    let len = (w as usize) * (h as usize) * 4;
    (vec![0u8; len], w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_new_decodes_embedded_layers() {
        assert!(
            FolderComposer::try_new().is_some(),
            "embedded folder PNG layers must decode"
        );
    }

    #[test]
    fn placeholder_folder_icon_is_valid_rgba() {
        let (rgba, w, h) = placeholder_folder_icon();
        assert!(w > 0 && h > 0);
        assert_eq!(rgba.len(), (w as usize) * (h as usize) * 4);
    }

    #[test]
    fn palette_variants_preserve_alpha_paper_and_media() {
        use crate::domain::folder_appearance::FolderAppearance;
        let original = FolderComposer::try_new().unwrap();
        for appearance in FolderAppearance::ALL.into_iter().skip(1) {
            let variant = FolderComposer::try_new()
                .unwrap()
                .with_appearance(appearance);
            assert_eq!(original.sheet_img, variant.sheet_img);
            for (before, after) in [
                (&original.back_img, &variant.back_img),
                (&original.front_img, &variant.front_img),
            ] {
                let before = before.to_rgba8();
                let after = after.to_rgba8();
                assert_ne!(before, after);
                for (a, b) in before.pixels().zip(after.pixels()) {
                    assert_eq!(a[3], b[3]);
                }
            }
            let media = [20, 80, 220, 255].repeat(32 * 32);
            let (before, w, h) = original.compose(&media, 32, 32).unwrap();
            let (after, sw, sh) = variant.compose(&media, 32, 32).unwrap();
            assert_eq!((w, h), (sw, sh));
            assert_ne!(before, after);
            let mut visible_media = 0;
            for (a, b) in before.chunks_exact(4).zip(after.chunks_exact(4)) {
                assert_eq!(a[3], b[3]);
                if a == [20, 80, 220, 255] {
                    assert_eq!(a, b);
                    visible_media += 1;
                }
            }
            assert!(visible_media > 0, "media must remain visible and unchanged");
            assert_ne!(original.compose_empty().0, variant.compose_empty().0);
        }
    }

    #[test]
    fn original_variant_is_unchanged_and_recoloring_rebuilds_existing_layers() {
        use crate::domain::folder_appearance::FolderAppearance;
        let original = FolderComposer::try_new().unwrap();
        let expected = original.compose_empty();
        let original = original.with_appearance(FolderAppearance::Original);
        assert_eq!(expected, original.compose_empty());
        let pastel = original.with_appearance(FolderAppearance::PastelYellow);
        assert_ne!(expected.0, pastel.compose_empty().0);
    }
}
