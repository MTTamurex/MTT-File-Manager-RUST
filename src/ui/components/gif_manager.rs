use eframe::egui;
use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

/// A single decoded frame of a GIF
#[derive(Clone)]
pub struct DecodedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub delay_ms: u64,
}

/// The state of a GIF being decoded
pub struct GifData {
    pub frames: Vec<DecodedFrame>,
    pub is_complete: bool,
    pub generation: usize,
}

impl GifData {
    fn new(generation: usize) -> Self {
        Self {
            frames: Vec::new(),
            is_complete: false,
            generation,
        }
    }
}

pub struct GifManager {
    cache: LruCache<PathBuf, Arc<Mutex<GifData>>>,
    current_generation: Arc<AtomicUsize>,
    ui_ctx: egui::Context,
}

impl GifManager {
    pub fn new(ui_ctx: egui::Context) -> Self {
        Self {
            cache: LruCache::new(NonZeroUsize::new(10).unwrap()),
            current_generation: Arc::new(AtomicUsize::new(0)),
            ui_ctx,
        }
    }

    /// Requests a GIF to be loaded. Returns the Arc<Mutex<GifData>> for the GIF.
    pub fn request_load(&mut self, path: &Path) -> Arc<Mutex<GifData>> {
        if let Some(data) = self.cache.get(path) {
            return data.clone();
        }

        let generation = self.current_generation.fetch_add(1, Ordering::SeqCst);
        let data = Arc::new(Mutex::new(GifData::new(generation)));
        self.cache.put(path.to_path_buf(), data.clone());

        let path_buf = path.to_path_buf();
        let data_clone = data.clone();
        let current_gen = self.current_generation.clone();
        let ui_ctx = self.ui_ctx.clone();

        thread::spawn(move || {
            if let Err(e) = Self::decode_worker(path_buf, data_clone, generation, current_gen, ui_ctx) {
                eprintln!("GifWorker error: {}", e);
            }
        });

        data
    }

    fn decode_worker(
        path: PathBuf,
        data: Arc<Mutex<GifData>>,
        generation: usize,
        _global_gen: Arc<AtomicUsize>,
        ui_ctx: egui::Context,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file = std::fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        let decoder = GifDecoder::new(reader)?;
        let frames = decoder.into_frames();

        for (i, frame_res) in frames.enumerate() {
            // Check if this task is still relevant
            // We use a simple generation check: if we started a newer load globally, 
            // and this one isn't the one being tracked, we might want to stop.
            // Actually, we only care if THIS path was re-requested or cleared.
            // For simplicity, we just check if the generation has moved too far 
            // or if we should just keep going? 
            // Better: use a weak ref or just let the cache handle it.
            // If the user selects a new GIF, this one might still be in cache.
            
            let frame = frame_res?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let delay_ms = if denominator == 0 {
                100
            } else {
                (numerator as u64) / (denominator as u64)
            };

            let buffer = frame.into_buffer();
            let (orig_w, orig_h) = buffer.dimensions();
            
            // Resize if too large (max 512px)
            let (w, h, rgba) = if orig_w > 512 || orig_h > 512 {
                let img = image::DynamicImage::ImageRgba8(buffer);
                let resized = img.thumbnail(512, 512);
                let rb = resized.to_rgba8();
                (rb.width(), rb.height(), rb.into_raw())
            } else {
                (orig_w, orig_h, buffer.into_raw())
            };

            {
                let mut d = data.lock().unwrap();
                // Check generation again inside lock to be safe
                if d.generation != generation {
                    return Ok(());
                }
                
                d.frames.push(DecodedFrame {
                    rgba,
                    width: w,
                    height: h,
                    delay_ms,
                });
                
                // If it's the first frame, request repaint immediately
                if i == 0 {
                    ui_ctx.request_repaint();
                }
            }

            // Throttle decoding slightly to avoid CPU hogging
            if i % 5 == 0 {
                thread::sleep(Duration::from_millis(5));
            }
            
            // Limit total frames to avoid OOM for crazy GIFs
            if i > 500 {
                break;
            }
        }

        let mut d = data.lock().unwrap();
        if d.generation == generation {
            d.is_complete = true;
            ui_ctx.request_repaint();
        }

        Ok(())
    }
}
