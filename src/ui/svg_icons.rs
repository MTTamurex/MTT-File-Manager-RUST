use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use lru::LruCache;
use std::num::NonZeroUsize;

/// Manages SVG icon loading and caching
pub struct SvgIconManager {
    /// Cache of rendered textures keyed by (icon_name, size, color) - LRU bounded to 200 entries
    cache: LruCache<(String, u32, [u8; 4]), TextureHandle>,
}

impl SvgIconManager {
    /// Create a new SvgIconManager
    pub fn new() -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(200).unwrap()),
        }
    }

    /// Get or create a texture for the specified icon
    /// Color is now part of the cache key to support toggle states
    pub fn get_icon(
        &mut self,
        ctx: &egui::Context,
        icon_name: &str,
        size: u32,
        color: [u8; 4],
    ) -> Option<TextureHandle> {
        // Include color in cache key for proper toggle state rendering
        let cache_key = (icon_name.to_string(), size, color);

        // Return cached texture if available
        if let Some(texture) = self.cache.get(&cache_key) {
            return Some(texture.clone());
        }

        // Load SVG from embedded assets
        let svg_data = crate::embedded_assets::get_icon(icon_name)?;
        let image = render_svg_to_image(svg_data, size, color)?;

        // Create texture with unique name including color
        let texture = ctx.load_texture(
            format!(
                "icon_{}_{}_{:02x}{:02x}{:02x}",
                icon_name, size, color[0], color[1], color[2]
            ),
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

/// Render an SVG file to a ColorImage at the specified size
fn render_svg_to_image(svg_data: &[u8], size: u32, color: [u8; 4]) -> Option<ColorImage> {
    // Parse SVG from embedded bytes
    let svg_str = std::str::from_utf8(svg_data).ok()?;

    // Parse SVG
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(&svg_str, &opt).ok()?;

    // Calculate scale to fit desired size
    let svg_size = tree.size();
    let scale_x = size as f32 / svg_size.width();
    let scale_y = size as f32 / svg_size.height();
    let scale = scale_x.min(scale_y);

    // Create pixmap for rendering
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;

    // Clear with transparent background
    pixmap.fill(tiny_skia::Color::TRANSPARENT);

    // Calculate offset to center the icon
    let scaled_w = svg_size.width() * scale;
    let scaled_h = svg_size.height() * scale;
    let offset_x = (size as f32 - scaled_w) / 2.0;
    let offset_y = (size as f32 - scaled_h) / 2.0;

    // Create transform with scale and offset
    let transform =
        tiny_skia::Transform::from_scale(scale, scale).post_translate(offset_x, offset_y);

    // Render SVG to pixmap
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Apply color tint - replace all non-transparent pixels with the given color
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

    // Convert to egui ColorImage
    let size_usize = [size as usize, size as usize];
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

    // Render at 2x resolution for HiDPI quality
    let render_size = (size * 2.0) as u32;

    // Aloca espaço para o botão (tamanho do ícone + padding implícito se desejar,
    // mas aqui mantemos 'size' para consistência layout)
    // Para um botão mais clicável, adicionamos um padding leve na área de interação
    let padding = 4.0;
    let button_size = egui::vec2(size + padding * 2.0, size + padding * 2.0);

    let (rect, response) = ui.allocate_exact_size(button_size, egui::Sense::click());

    // Desenha background se hover
    if response.hovered() {
        let bg_color = if ui.visuals().dark_mode {
            egui::Color32::from_white_alpha(30)
        } else {
            egui::Color32::from_black_alpha(20)
        };
        ui.painter().rect_filled(rect, 4.0, bg_color);
    }

    // Desenha ícone
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

    // Render at 2x resolution for HiDPI quality
    let render_size = (size * 2.0) as u32;

    if let Some(texture) = icon_manager.get_icon(ui.ctx(), icon_name, render_size, color) {
        ui.image(egui::load::SizedTexture::new(
            texture.id(),
            egui::vec2(size, size), // Display at requested size
        ));
    } else {
        ui.label("?");
    }
}
