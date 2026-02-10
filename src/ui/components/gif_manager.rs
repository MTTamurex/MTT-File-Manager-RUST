use eframe::egui;
use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    pub cancelled: Arc<AtomicBool>,
    pub total_bytes: usize,
    pub last_used: Instant,
}

impl GifData {
    fn new(generation: usize) -> Self {
        Self {
            frames: Vec::new(),
            is_complete: false,
            generation,
            cancelled: Arc::new(AtomicBool::new(false)),
            total_bytes: 0,
            last_used: Instant::now(),
        }
    }
}

pub struct GifManager {
    cache: LruCache<PathBuf, Arc<Mutex<GifData>>>,
    current_generation: Arc<AtomicUsize>,
    ui_ctx: egui::Context,
    max_memory_bytes: usize,
    /// PERFORMANCE: Running total of memory usage across all cached GIFs.
    /// Updated atomically when frames are added/removed, avoiding O(N) iteration.
    running_total_bytes: Arc<AtomicUsize>,
}

impl GifManager {
    pub fn new(ui_ctx: egui::Context) -> Self {
        Self {
            // Increase cache slots but use memory-based eviction logic
            cache: LruCache::new(NonZeroUsize::new(100).expect("gif cache size must be non-zero")),
            current_generation: Arc::new(AtomicUsize::new(0)),
            ui_ctx,
            max_memory_bytes: 150 * 1024 * 1024, // 150 MB
            running_total_bytes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Requests a GIF to be loaded. Returns the Arc<Mutex<GifData>> for the GIF.
    pub fn request_load(&mut self, path: &Path) -> Arc<Mutex<GifData>> {
        if let Some(data) = self.cache.get(path) {
            let d_clone = data.clone();
            {
                let mut d = data.lock().unwrap();
                d.last_used = Instant::now();
            }
            return d_clone;
        }

        // Cleanup before adding new
        self.cleanup(false);

        let generation = self.current_generation.fetch_add(1, Ordering::SeqCst);
        let data = Arc::new(Mutex::new(GifData::new(generation)));
        self.cache.put(path.to_path_buf(), data.clone());

        let path_buf = path.to_path_buf();
        let data_clone = data.clone();
        let current_gen = self.current_generation.clone();
        let ui_ctx = self.ui_ctx.clone();
        let running_total = self.running_total_bytes.clone();

        thread::spawn(move || {
            if let Err(e) = Self::decode_worker(
                path_buf,
                data_clone,
                generation,
                current_gen,
                ui_ctx,
                running_total,
            ) {
                eprintln!("GifWorker error: {}", e);
            }
        });

        data
    }

    /// Periodic or manual cleanup of the GIF cache
    /// PERFORMANCE: Uses running_total_bytes for O(1) memory check instead of O(N) iteration
    pub fn cleanup(&mut self, force_all: bool) {
        if force_all {
            for (_, data) in self.cache.iter() {
                let d = data.lock().unwrap();
                d.cancelled.store(true, Ordering::SeqCst);
            }
            self.cache.clear();
            self.running_total_bytes.store(0, Ordering::SeqCst);
            return;
        }

        let now = Instant::now();
        let ttl = Duration::from_secs(30);

        // 1. TTL Cleanup - collect expired paths without holding locks
        let mut to_remove = Vec::new();
        for (path, data) in self.cache.iter() {
            let d = data.lock().unwrap();
            if now.duration_since(d.last_used) > ttl {
                d.cancelled.store(true, Ordering::SeqCst);
                to_remove.push((path.clone(), d.total_bytes));
            }
        }
        for (path, bytes) in to_remove {
            self.cache.pop(&path);
            self.running_total_bytes.fetch_sub(bytes, Ordering::SeqCst);
        }

        // 2. Memory-based LRU Cleanup - O(1) check using running total
        while self.running_total_bytes.load(Ordering::SeqCst) > self.max_memory_bytes
            && !self.cache.is_empty()
        {
            // Pop least recently used
            if let Some((_, data)) = self.cache.pop_lru() {
                let d = data.lock().unwrap();
                d.cancelled.store(true, Ordering::SeqCst);
                self.running_total_bytes
                    .fetch_sub(d.total_bytes, Ordering::SeqCst);
            }
        }
    }

    fn decode_worker(
        path: PathBuf,
        data: Arc<Mutex<GifData>>,
        generation: usize,
        _global_gen: Arc<AtomicUsize>,
        ui_ctx: egui::Context,
        running_total: Arc<AtomicUsize>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file = std::fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        let decoder = GifDecoder::new(reader)?;
        let frames = decoder.into_frames();

        let cancelled = {
            let d = data.lock().unwrap();
            d.cancelled.clone()
        };

        for (i, frame_res) in frames.enumerate() {
            // Check for cancellation
            if cancelled.load(Ordering::SeqCst) {
                return Ok(());
            }

            // Automatic cancellation if nobody is watching
            // Strong count 1 means only this thread's Arc (the `data` argument) is left
            if Arc::strong_count(&data) <= 1 {
                return Ok(());
            }

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

            let frame_bytes = rgba.len();

            {
                let mut d = data.lock().unwrap();
                // Check generation and cancellation inside lock
                if d.generation != generation || d.cancelled.load(Ordering::SeqCst) {
                    return Ok(());
                }

                d.total_bytes += frame_bytes;
                // PERFORMANCE: Update running total atomically for O(1) memory tracking
                running_total.fetch_add(frame_bytes, Ordering::SeqCst);
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
        if d.generation == generation && !d.cancelled.load(Ordering::SeqCst) {
            d.is_complete = true;
            ui_ctx.request_repaint();
        }

        Ok(())
    }
}
