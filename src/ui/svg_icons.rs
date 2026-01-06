use std::collections::HashMap;
use std::path::Path;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

/// Manages SVG icon loading and caching
pub struct SvgIconManager {
    /// Cache of rendered textures keyed by (icon_name, size, color)
    cache: HashMap<(String, u32, [u8; 4]), TextureHandle>,
    /// Base path for icon assets
    icons_dir: std::path::PathBuf,
}

impl SvgIconManager {
    /// Create a new SvgIconManager with the given icons directory
    pub fn new(icons_dir: impl AsRef<Path>) -> Self {
        Self {
            cache: HashMap::new(),
            icons_dir: icons_dir.as_ref().to_path_buf(),
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
        
        // Load and render the SVG
        let svg_path = self.icons_dir.join(format!("{}.svg", icon_name));
        let image = render_svg_to_image(&svg_path, size, color)?;
        
        // Create texture with unique name including color
        let texture = ctx.load_texture(
            format!("icon_{}_{}_{:02x}{:02x}{:02x}", icon_name, size, color[0], color[1], color[2]),
            image,
            TextureOptions::LINEAR,
        );
        
        // Cache and return
        self.cache.insert(cache_key, texture.clone());
        Some(texture)
    }
    
    /// Clear the texture cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Render an SVG file to a ColorImage at the specified size
fn render_svg_to_image(svg_path: &Path, size: u32, color: [u8; 4]) -> Option<ColorImage> {
    // Read SVG file
    let svg_data = std::fs::read_to_string(svg_path).ok()?;
    
    // Parse SVG
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(&svg_data, &opt).ok()?;
    
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
    let transform = tiny_skia::Transform::from_scale(scale, scale)
        .post_translate(offset_x, offset_y);
    
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
    Some(ColorImage::from_rgba_unmultiplied(size_usize, pixmap.data()))
}

/// Convenience function to render an SVG icon as a simple button
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
    
    if let Some(texture) = icon_manager.get_icon(ui.ctx(), icon_name, size as u32, color) {
        let response = ui.add(
            egui::ImageButton::new(egui::load::SizedTexture::new(
                texture.id(),
                egui::vec2(size, size),
            ))
            .frame(false)
        );
        
        if !tooltip.is_empty() {
            response.clone().on_hover_text(tooltip);
        }
        
        response
    } else {
        ui.add(egui::Button::new("?").min_size(egui::vec2(size, size)))
    }
}

/// Draw an icon as a simple image (no button behavior)
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
    
    if let Some(texture) = icon_manager.get_icon(ui.ctx(), icon_name, size as u32, color) {
        ui.image(egui::load::SizedTexture::new(
            texture.id(),
            egui::vec2(size, size),
        ));
    } else {
        ui.label("?");
    }
}
