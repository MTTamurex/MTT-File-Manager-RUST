use eframe::egui;
use std::time::{Duration, Instant};
use image::AnimationDecoder;
use image::codecs::gif::GifDecoder;

/// Represents a single frame of an animated GIF.
pub struct GifFrame {
    pub texture: egui::TextureHandle,
    pub delay: Duration,
}

/// Handles the playback logic for animated GIFs.
pub struct GifPlayer {
    pub frames: Vec<GifFrame>,
    pub current_frame: usize,
    pub last_update: Instant,
    pub total_duration: Duration,
}

impl GifPlayer {
    /// Tenta carregar um GIF do caminho fornecido.
    pub fn load(ctx: &egui::Context, path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let decoder = GifDecoder::new(reader)?;
        let frames_result = decoder.into_frames().collect_frames()?;

        let mut gif_frames = Vec::new();
        let mut total_duration = Duration::ZERO;

        for (i, frame) in frames_result.into_iter().enumerate() {
            let (numerator, denominator): (u32, u32) = frame.delay().numer_denom_ms();
            let delay = if denominator == 0 {
                Duration::from_millis(100) // Fallback
            } else {
                Duration::from_millis((numerator as u64) / (denominator as u64))
            };

            let buffer = frame.into_buffer();
            let (width, height) = buffer.dimensions();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                buffer.as_flat_samples().as_slice(),
            );

            let texture = ctx.load_texture(
                format!("gif_frame_{}_{}", path.display(), i),
                color_image,
                Default::default(),
            );

            total_duration += delay;
            gif_frames.push(GifFrame { texture, delay });
        }

        if gif_frames.is_empty() {
            return Err("GIF has no frames".into());
        }

        Ok(Self {
            frames: gif_frames,
            current_frame: 0,
            last_update: Instant::now(),
            total_duration,
        })
    }

    /// Atualiza o estado da animação e retorna se o frame mudou.
    pub fn update(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update);
        let current_delay = self.frames[self.current_frame].delay;

        if elapsed >= current_delay {
            self.current_frame = (self.current_frame + 1) % self.frames.len();
            self.last_update = now;
            // Solicita o próximo repaint exatamente no tempo do próximo delay
            ctx.request_repaint_after(self.frames[self.current_frame].delay);
        } else {
            // Ainda falta tempo para o próximo frame
            ctx.request_repaint_after(current_delay - elapsed);
        }
    }

    pub fn current_texture(&self) -> &egui::TextureHandle {
        &self.frames[self.current_frame].texture
    }
}

/// Enum principal para previews de mídia.
pub enum MediaPreview {
    StaticImage(egui::TextureHandle),
    AnimatedGif(GifPlayer),
    Video, // Placeholder para futura implementação (FFmpeg/GStreamer)
}

impl MediaPreview {
    pub fn show(&mut self, ui: &mut egui::Ui) -> egui::Response {
        match self {
            MediaPreview::StaticImage(texture) => {
                let max_size = egui::vec2(ui.available_width() - 16.0, ui.available_width() - 16.0);
                ui.add(
                    egui::Image::new(&*texture)
                        .max_size(max_size)
                        .shrink_to_fit()
                )
            }
            MediaPreview::AnimatedGif(player) => {
                player.update(ui.ctx());
                let texture = player.current_texture();
                let max_size = egui::vec2(ui.available_width() - 16.0, ui.available_width() - 16.0);
                
                ui.add(
                    egui::Image::new(texture)
                        .max_size(max_size)
                        .shrink_to_fit()
                )
            }
            MediaPreview::Video => {
                ui.vertical_centered(|ui| {
                    ui.label("📹 Preview de vídeo não disponível");
                    ui.label("(FFmpeg integration pending)");
                }).response
            }
        }
    }
}
