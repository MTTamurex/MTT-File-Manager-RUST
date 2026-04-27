use eframe::egui;
use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;
use lru::LruCache;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Maximum number of concurrent GIF decode workers.
/// Prevents unbounded thread creation when many GIFs are visible simultaneously.
const GIF_DECODE_WORKERS: usize = 3;
const GIF_MAX_MEMORY_BYTES: usize = 150 * 1024 * 1024;
const GIF_MAX_BYTES_PER_ENTRY: usize = 24 * 1024 * 1024;
const GIF_MAX_FRAMES: usize = 500;

/// Job sent to a GIF decode worker thread.
struct GifDecodeJob {
    path: PathBuf,
    data: Arc<Mutex<GifData>>,
    generation: usize,
    global_gen: Arc<AtomicUsize>,
    ui_ctx: egui::Context,
    running_total: Arc<AtomicUsize>,
    max_memory_bytes: usize,
    max_gif_bytes: usize,
}

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
    /// Bounded channel sender for GIF decode jobs.
    /// Workers are spawned once in `new()` and loop on the receiver.
    job_sender: crossbeam_channel::Sender<GifDecodeJob>,
}

impl GifManager {
    pub fn new(ui_ctx: egui::Context) -> Self {
        let (job_sender, job_receiver) = crossbeam_channel::bounded::<GifDecodeJob>(16);

        // Spawn fixed pool of decode workers
        for worker_id in 0..GIF_DECODE_WORKERS {
            let rx = job_receiver.clone();
            let _ = std::thread::Builder::new()
                .name(format!("gif-decode-{}", worker_id))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        if let Err(e) = Self::decode_worker(
                            job.path,
                            job.data,
                            job.generation,
                            job.global_gen,
                            job.ui_ctx,
                            job.running_total,
                            job.max_memory_bytes,
                            job.max_gif_bytes,
                        ) {
                            log::error!("GifWorker-{}: {}", worker_id, e);
                        }
                    }
                });
        }

        Self {
            // Increase cache slots but use memory-based eviction logic
            cache: LruCache::new(NonZeroUsize::new(100).expect("gif cache size must be non-zero")),
            current_generation: Arc::new(AtomicUsize::new(0)),
            ui_ctx,
            max_memory_bytes: GIF_MAX_MEMORY_BYTES,
            running_total_bytes: Arc::new(AtomicUsize::new(0)),
            job_sender,
        }
    }

    /// Requests a GIF to be loaded. Returns the Arc<Mutex<GifData>> for the GIF.
    pub fn request_load(&mut self, path: &Path) -> Arc<Mutex<GifData>> {
        if let Some(data) = self.cache.get(path) {
            let d_clone = data.clone();
            {
                let mut d = data.lock();
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
        let max_memory_bytes = self.max_memory_bytes;

        // Send to bounded worker pool instead of spawning unbounded threads.
        // If the pool is full, the oldest pending job is implicitly throttled.
        let _ = self.job_sender.try_send(GifDecodeJob {
            path: path_buf,
            data: data_clone,
            generation,
            global_gen: current_gen,
            ui_ctx,
            running_total,
            max_memory_bytes,
            max_gif_bytes: GIF_MAX_BYTES_PER_ENTRY,
        });

        data
    }

    /// Periodic or manual cleanup of the GIF cache
    /// PERFORMANCE: Uses running_total_bytes for O(1) memory check instead of O(N) iteration
    pub fn cleanup(&mut self, force_all: bool) {
        if force_all {
            for (_, data) in self.cache.iter() {
                let d = data.lock();
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
            let d = data.lock();
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
                let d = data.lock();
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
        max_memory_bytes: usize,
        max_gif_bytes: usize,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file = std::fs::File::open(&path)?;
        let reader = std::io::BufReader::new(file);
        let decoder = GifDecoder::new(reader)?;
        let frames = decoder.into_frames();

        let cancelled = {
            let d = data.lock();
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
            let running_total_before = running_total.load(Ordering::SeqCst);

            {
                let mut d = data.lock();
                // Check generation and cancellation inside lock
                if d.generation != generation || d.cancelled.load(Ordering::SeqCst) {
                    return Ok(());
                }

                let has_visible_frames = !d.frames.is_empty();
                let exceeds_per_gif_budget = d.total_bytes.saturating_add(frame_bytes) > max_gif_bytes;
                let exceeds_global_budget = running_total_before.saturating_add(frame_bytes) > max_memory_bytes;

                if has_visible_frames && (exceeds_per_gif_budget || exceeds_global_budget) {
                    d.is_complete = true;
                    ui_ctx.request_repaint();
                    log::debug!(
                        "[GIF] Truncated decode for {:?}: frames={} total_bytes={} next_frame={} global_total={} exceeds_per_gif={} exceeds_global={}",
                        path,
                        d.frames.len(),
                        d.total_bytes,
                        frame_bytes,
                        running_total_before,
                        exceeds_per_gif_budget,
                        exceeds_global_budget
                    );
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
            if i >= GIF_MAX_FRAMES {
                break;
            }
        }

        let mut d = data.lock();
        if d.generation == generation && !d.cancelled.load(Ordering::SeqCst) {
            d.is_complete = true;
            ui_ctx.request_repaint();
        }

        Ok(())
    }
}
