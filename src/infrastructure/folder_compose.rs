//! Custom folder cover composition
//!
//! Composes a folder preview by overlaying three layers:
//! 1. `folder_back_512.png`  — folder silhouette background
//! 2. Media thumbnail        — content preview (image/video from inside the folder)
//! 3. `folder_front_512.png` — folder front flap overlay
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
//! - Each composition takes ~1-2ms (resize + two alpha-blend passes on 256×256)
//! - Much faster than Shell API (20-200ms per folder via COM interop)

use image::{imageops, DynamicImage, RgbaImage};
use std::io::Cursor;

/// Output width — both layers are scaled to this width (preserving aspect ratio).
const OUTPUT_W: u32 = 256;

/// Content thumbnail area (pixel coords at OUTPUT_W scale).
/// These define the "window" where the media preview sits between back and front.
/// back_512 at 256px = 256×173, front_512 at 256px = 256×112.
/// The visible gap where the thumbnail peeks out is roughly:
///   top = back_top (bottom - 173) + small margin
///   bottom = front_top - small margin
/// Tunable constants:
const CONTENT_MARGIN_LEFT: u32 = 10;
const CONTENT_MARGIN_RIGHT: u32 = 10;
const CONTENT_MARGIN_TOP: u32 = 30;
const CONTENT_MARGIN_BOTTOM: u32 = 0;

/// Pre-decoded folder composition layers.
///
/// Created once at app startup, shared across all folder preview worker
/// threads via `Arc<FolderComposer>`. Thread-safe because all fields
/// are immutable after construction.
pub struct FolderComposer {
    /// Folder background layer, scaled to OUTPUT_W wide
    back: RgbaImage,
    /// Folder front overlay, scaled to OUTPUT_W wide
    front: RgbaImage,
    /// Paper sheet fallback, pre-scaled to fit the content gap
    paper_sheet: RgbaImage,
    /// Canvas height (max of back/front heights, both bottom-aligned)
    canvas_h: u32,
    /// Y position of back layer on canvas (bottom-aligned)
    back_y: u32,
    /// Y position of front layer on canvas (bottom-aligned)
    front_y: u32,
}

impl FolderComposer {
    /// Decodes and pre-scales the embedded folder PNG layers.
    ///
    /// Called ONCE at app startup. Panics if the embedded PNGs are invalid
    /// (should never happen since they're compile-time embedded).
    pub fn new() -> Self {
        let back_img = image::load(
            Cursor::new(crate::embedded_assets::FOLDER_BACK_PNG),
            image::ImageFormat::Png,
        )
        .expect("Failed to decode embedded folder_back.png");

        let front_img = image::load(
            Cursor::new(crate::embedded_assets::FOLDER_FRONT_PNG),
            image::ImageFormat::Png,
        )
        .expect("Failed to decode embedded folder_front.png");

        // Scale both to OUTPUT_W wide, preserving aspect ratio.
        // Since both source PNGs are 512px wide, this halves them uniformly.
        let back = back_img
            .resize(OUTPUT_W, u32::MAX, imageops::FilterType::CatmullRom)
            .to_rgba8();
        let front = front_img
            .resize(OUTPUT_W, u32::MAX, imageops::FilterType::CatmullRom)
            .to_rgba8();

        // Canvas height = tallest layer. Both are bottom-aligned.
        let canvas_h = back.height().max(front.height());
        let back_y = canvas_h.saturating_sub(back.height());
        let front_y = canvas_h.saturating_sub(front.height());

        // Pre-scale paper_sheet to fit the content gap width.
        let gap_w = OUTPUT_W.saturating_sub(CONTENT_MARGIN_LEFT + CONTENT_MARGIN_RIGHT);
        let sheet_img = image::load(
            Cursor::new(crate::embedded_assets::PAPER_SHEET_PNG),
            image::ImageFormat::Png,
        )
        .expect("Failed to decode embedded paper_sheet.png");
        let paper_sheet = sheet_img
            .resize(gap_w, u32::MAX, imageops::FilterType::CatmullRom)
            .to_rgba8();

        log::info!(
            "[FOLDER COMPOSE] Layers decoded — back: {}×{} (y={}), front: {}×{} (y={}), canvas: {}×{}",
            back.width(), back.height(), back_y,
            front.width(), front.height(), front_y,
            OUTPUT_W, canvas_h,
        );

        Self {
            back,
            front,
            paper_sheet,
            canvas_h,
            back_y,
            front_y,
        }
    }

    /// Composes a folder cover with no media content.
    ///
    /// Uses the pre-scaled `paper_sheet.png` as the middle layer so empty
    /// folders still show a distinct interior instead of a blank silhouette.
    pub fn compose_empty(&self) -> (Vec<u8>, u32, u32) {
        let sheet = &self.paper_sheet;
        let result = self.compose(sheet.as_raw(), sheet.width(), sheet.height());
        // compose() only returns None on malformed input — paper_sheet is always valid.
        result.unwrap_or_else(|| {
            // Absolute fallback: bare back + front (should never happen).
            let mut canvas = RgbaImage::new(OUTPUT_W, self.canvas_h);
            let bx = OUTPUT_W.saturating_sub(self.back.width()) / 2;
            imageops::overlay(&mut canvas, &self.back, bx as i64, self.back_y as i64);
            let fx = OUTPUT_W.saturating_sub(self.front.width()) / 2;
            imageops::overlay(&mut canvas, &self.front, fx as i64, self.front_y as i64);
            let w = canvas.width();
            let h = canvas.height();
            (canvas.into_raw(), w, h)
        })
    }

    /// Composes a folder cover image from a media thumbnail.
    ///
    /// Layers: `folder_back` → content thumbnail (centered in gap) → `folder_front`
    /// Both back and front are bottom-aligned on the canvas.
    ///
    /// Returns `Some((rgba_data, width, height))` or `None` if input is invalid.
    pub fn compose(
        &self,
        content_rgba: &[u8],
        content_w: u32,
        content_h: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        // Validate input
        let expected = (content_w as usize)
            .checked_mul(content_h as usize)?
            .checked_mul(4)?;
        if content_rgba.len() != expected || content_w == 0 || content_h == 0 {
            return None;
        }

        // 1. Transparent canvas
        let mut canvas = RgbaImage::new(OUTPUT_W, self.canvas_h);

        // 2. Back layer (bottom-aligned, centered horizontally)
        let bx = OUTPUT_W.saturating_sub(self.back.width()) / 2;
        imageops::overlay(&mut canvas, &self.back, bx as i64, self.back_y as i64);

        // 3. Content thumbnail — fills the gap width, crops from bottom (top-aligned).
        //    Like Windows: thumbnail is scaled to fill the gap width, then only
        //    the top portion is shown (bottom is cut off by the front layer).
        let gap_top = self.back_y + CONTENT_MARGIN_TOP;
        let gap_bottom = self.front_y.saturating_sub(CONTENT_MARGIN_BOTTOM);
        let gap_h = gap_bottom.saturating_sub(gap_top);
        let gap_w = OUTPUT_W.saturating_sub(CONTENT_MARGIN_LEFT + CONTENT_MARGIN_RIGHT);

        if gap_h > 0 && gap_w > 0 {
            let content_img =
                RgbaImage::from_raw(content_w, content_h, content_rgba.to_vec())?;
            let content_dyn = DynamicImage::ImageRgba8(content_img);

            // Scale to fill gap width (extends downward past front — covered naturally by front overlay)
            let scale = gap_w as f32 / content_w as f32;
            let scaled_h = (content_h as f32 * scale).round() as u32;
            let scaled = content_dyn
                .resize_exact(gap_w, scaled_h, imageops::FilterType::CatmullRom)
                .to_rgba8();

            // Crop to canvas height minus gap_top to avoid drawing below canvas.
            // The front layer overlays on top and naturally covers the lower portion.
            let max_h = self.canvas_h.saturating_sub(gap_top);
            let crop_h = scaled_h.min(max_h);
            let cropped = imageops::crop_imm(&scaled, 0, 0, gap_w, crop_h).to_image();

            // Center horizontally, top-aligned at gap_top
            let cx = CONTENT_MARGIN_LEFT + gap_w.saturating_sub(cropped.width()) / 2;
            let cy = gap_top;
            imageops::overlay(&mut canvas, &cropped, cx as i64, cy as i64);
        }

        // 4. Front layer (bottom-aligned, centered horizontally)
        let fx = OUTPUT_W.saturating_sub(self.front.width()) / 2;
        imageops::overlay(&mut canvas, &self.front, fx as i64, self.front_y as i64);

        let w = canvas.width();
        let h = canvas.height();
        Some((canvas.into_raw(), w, h))
    }
}
