/// Minimal image viewer for testing—no cache, no prefetch, no sequence.
/// Just: open window → load image → display → done.

use std::path::PathBuf;
use eframe::egui;

pub fn run_minimal(path: PathBuf) -> eframe::Result<()> {
    log::info!("[IMAGE-VIEWER-MINIMAL] start path='{}'", path.display());

    let title = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "Image".to_string());

    let viewport = eframe::egui::ViewportBuilder::default()
        .with_title(format!("Image: {}", title))
        .with_inner_size([1200.0, 850.0])
        .with_visible(true)  // ← Show immediately, no hide/show games
        .with_resizable(true)
        .with_decorations(true);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let image_data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(e) => {
            log::error!("[IMAGE-VIEWER-MINIMAL] failed to read file: {}", e);
            return Ok(());
        }
    };

    eframe::run_native(
        "Image Viewer",
        options,
        Box::new(move |cc| {
            let ctx = &cc.egui_ctx;
            let mut app = MinimalApp {
                image_bytes: image_data.clone(),
                texture: None,
                zoom: 1.0,
            };

            // Try to decode and register texture immediately
            if let Ok(img) = image::load_from_memory(&app.image_bytes) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                let tex = ctx.load_texture("main_image", pixels, Default::default());
                app.texture = Some(tex);
                log::info!("[IMAGE-VIEWER-MINIMAL] texture registered");
            }

            Ok(Box::new(app))
        }),
    )
}

pub struct MinimalApp {
    pub image_bytes: Vec<u8>,
    pub texture: Option<egui::TextureHandle>,
    pub zoom: f32,
}

impl eframe::App for MinimalApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tex) = &self.texture {
                let available_size = ui.available_size();
                let img_size = tex.size_vec2();
                let scale = (available_size.x / img_size.x)
                    .min(available_size.y / img_size.y)
                    .min(1.0);
                let display_size = img_size * scale * self.zoom;
                ui.image(egui::load::SizedTexture::new(tex.id(), display_size));
            } else {
                ui.label("Failed to load image");
            }
        });
    }
}
