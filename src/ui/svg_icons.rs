use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use lru::LruCache;
use std::num::NonZeroUsize;

/// M-7: u64 hash of (icon_name, size, effective_color, ppp_100).
/// Eliminates a String allocation on every cache hit.
/// Collision probability ≈ 8×10⁻¹² for a 200-entry cache.
#[inline]
fn svg_icon_key(icon_name: &str, size: u32, effective_color: [u8; 4], ppp_100: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    icon_name.hash(&mut h);
    size.hash(&mut h);
    effective_color.hash(&mut h);
    ppp_100.hash(&mut h);
    h.finish()
}

/// Manages SVG icon loading and caching
pub struct SvgIconManager {
    /// Cache of rendered textures keyed by a u64 hash of (icon_name, size, color, pixels_per_point)
    cache: LruCache<u64, TextureHandle>,
}

impl Default for SvgIconManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SvgIconManager {
    /// Create a new SvgIconManager
    pub fn new() -> Self {
        Self {
            cache: LruCache::new(
                NonZeroUsize::new(200).expect("svg icon cache size must be non-zero"),
            ),
        }
    }

    /// Get or create a texture for the specified icon
    pub fn get_icon(
        &mut self,
        ctx: &egui::Context,
        icon_name: &str,
        size: u32,
        color: [u8; 4],
    ) -> Option<TextureHandle> {
        // Check if icon should preserve original colors (duotone icons)
        // But only if NOT in disabled state (disabled uses alpha < 255 or specific gray)
        let is_duotone = matches!(
            icon_name,
            "copy" | "cut" | "paste" | "rename" | "folder_new"
        );
        // Disabled state is indicated by: alpha < 255 (e.g., [128, 128, 128, 180])
        let is_disabled = color[3] < 255;
        let preserve_colors = is_duotone && !is_disabled;

        let ppp = ctx.pixels_per_point();
        // M-7: hash key — no String alloc on cache hit
        // Duotone icons use the color to recolor black pixels, so include it in the key.
        let ppp_100 = (ppp * 100.0) as u32;
        let cache_key = svg_icon_key(icon_name, size, color, ppp_100);

        // Return cached texture if available
        if let Some(texture) = self.cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Load SVG from embedded assets
        let svg_data = crate::embedded_assets::get_icon(icon_name)?;
        let image = render_svg_to_image(svg_data, size, color, ppp, preserve_colors)?;

        // Create texture with unique name
        let color_str = format!("{:02x}{:02x}{:02x}", color[0], color[1], color[2]);

        let texture = ctx.load_texture(
            format!("icon_{}_{}_{}_{}", icon_name, size, ppp_100, color_str),
            image,
            TextureOptions::LINEAR,
        );

        // Cache and return
        self.cache.put(cache_key, texture.clone());
        Some(texture)
    }

    /// Clear the texture cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Render an SVG file to a ColorImage at the specified logical size, respecting pixels_per_point
fn render_svg_to_image(
    svg_data: &[u8],
    logical_size: u32,
    color: [u8; 4],
    ppp: f32,
    preserve_colors: bool,
) -> Option<ColorImage> {
    // Physical size for rendering - ensuring 1:1 pixel mapping
    let physical_size = (logical_size as f32 * ppp).round() as u32;
    if physical_size == 0 {
        return None;
    }

    // Parse SVG from embedded bytes
    let svg_str = std::str::from_utf8(svg_data).ok()?;

    // Parse SVG
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg_str, &opt).ok()?;

    // Calculate scale to fit exactly into physical pixels
    let svg_size = tree.size();
    let scale_x = physical_size as f32 / svg_size.width();
    let scale_y = physical_size as f32 / svg_size.height();
    let scale = scale_x.min(scale_y);

    // Create pixmap for rendering at exact physical resolution
    let mut pixmap = tiny_skia::Pixmap::new(physical_size, physical_size)?;

    // Clear with transparent background
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    // Calculate offset to center the icon in the physical pixmap
    let scaled_w = svg_size.width() * scale;
    let scaled_h = svg_size.height() * scale;
    let offset_x = (physical_size as f32 - scaled_w) / 2.0;
    let offset_y = (physical_size as f32 - scaled_h) / 2.0;

    // Create transform with scale and offset
    let transform =
        tiny_skia::Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);

    // Render SVG to pixmap
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Apply color tint
    if !preserve_colors {
        // Monochrome: replace ALL pixel colors with target color
        let pixels = pixmap.data_mut();
        for chunk in pixels.chunks_exact_mut(4) {
            let alpha = chunk[3];
            if alpha > 0 {
                chunk[0] = color[0];
                chunk[1] = color[1];
                chunk[2] = color[2];
                chunk[3] = ((alpha as u32 * color[3] as u32) / 255) as u8;
            }
        }
    } else {
        // Duotone: replace only dark/black pixels with target color,
        // keeping the accent-colored (blue) pixels intact.
        let pixels = pixmap.data_mut();
        for chunk in pixels.chunks_exact_mut(4) {
            let alpha = chunk[3];
            if alpha > 0 {
                // In premultiplied alpha, black pixels have near-zero RGB.
                // Accent blue (#7AB8FF) pixels have significant RGB values.
                let rgb_sum = chunk[0] as u32 + chunk[1] as u32 + chunk[2] as u32;
                if rgb_sum < (alpha as u32) / 2 {
                    chunk[0] = color[0];
                    chunk[1] = color[1];
                    chunk[2] = color[2];
                }
            }
        }
    }

    // Convert to egui ColorImage
    let size_usize = [physical_size as usize, physical_size as usize];
    Some(ColorImage::from_rgba_unmultiplied(
        size_usize,
        pixmap.data(),
    ))
}

/// Convenience function to render an SVG icon as a simple button
/// Renders at 2x resolution for HiDPI quality
pub fn icon_button(
    ui: &mut egui::Ui,
    icon_manager: &mut SvgIconManager,
    icon_name: &str,
    size: f32,
    tooltip: &str,
) -> egui::Response {
    let color = if ui.visuals().dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };

    // Use 1:1 physical rendering (logical_size * ppp)
    let render_size = size as u32;

    // Allocate space for the button (icon size + implicit padding if desired,
    // but here we keep 'size' for layout consistency)
    // For a more clickable button, we add light padding to the interaction area
    let padding = 4.0;
    let button_size = egui::vec2(size + padding * 2.0, size + padding * 2.0);

    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());

    // Draw background on hover
    if response.hovered() {
        let bg_color = if ui.visuals().dark_mode {
            egui::Color32::from_white_alpha(30)
        } else {
            egui::Color32::from_black_alpha(20)
        };
        ui.painter().rect_filled(rect, 4.0, bg_color);
    }

    // Draw icon
    if let Some(texture) = icon_manager.get_icon(ui.ctx(), icon_name, render_size, color) {
        let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(size, size));
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        // Fallback
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "?",
            egui::FontId::proportional(size * 0.8),
            ui.visuals().text_color(),
        );
    }

    if !tooltip.is_empty() {
        response.clone().on_hover_text(tooltip)
    } else {
        response
    }
}

/// Draw an icon as a simple image (no button behavior)
/// Renders at 2x resolution for HiDPI quality
pub fn icon_image(
    ui: &mut egui::Ui,
    icon_manager: &mut SvgIconManager,
    icon_name: &str,
    size: f32,
) {
    let color = if ui.visuals().dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };

    // Use 1:1 physical rendering (logical_size * ppp)
    let render_size = size as u32;

    if let Some(texture) = icon_manager.get_icon(ui.ctx(), icon_name, render_size, color) {
        ui.image(egui::load::SizedTexture::new(
            texture.id(),
            egui::vec2(size, size), // Display at requested size
        ));
    } else {
        ui.label("?");
    }
}
