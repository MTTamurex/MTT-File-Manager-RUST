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
const GIF_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const GIF_PREVIEW_MAX_DIMENSION: u32 = 384;
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
    pub rgba: Option<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub original_width: u32,
    pub original_height: u32,
    pub delay_ms: u64,
}

/// The state of a GIF being decoded
pub struct GifData {
    pub frames: Vec<DecodedFrame>,
    pub is_complete: bool,
    pub generation: usize,
    pub cancelled: Arc<AtomicBool>,
    /// CPU-side RGBA staging bytes that have not yet been uploaded to GPU.
    pub total_bytes: usize,
    /// Estimated texture footprint for decoded frames. This does not decrease
    /// after upload and keeps pathological GIFs from moving OOM risk to VRAM.
    pub retained_frame_bytes: usize,
    pub last_used: Instant,
    running_total: Arc<AtomicUsize>,
}

impl GifData {
    fn new(generation: usize, running_total: Arc<AtomicUsize>) -> Self {
        Self {
            frames: Vec::new(),
            is_complete: false,
            generation,
            cancelled: Arc::new(AtomicBool::new(false)),
            total_bytes: 0,
            retained_frame_bytes: 0,
            last_used: Instant::now(),
            running_total,
        }
    }

    pub fn take_frame_rgba(&mut self, frame_index: usize) -> Option<Vec<u8>> {
        let rgba = self.frames.get_mut(frame_index)?.rgba.take()?;
        let bytes = rgba.len();
        self.total_bytes = self.total_bytes.saturating_sub(bytes);
        atomic_saturating_sub(&self.running_total, bytes);
        Some(rgba)
    }
}

fn atomic_saturating_sub(value: &AtomicUsize, amount: usize) {
    let _ = value.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        Some(current.saturating_sub(amount))
    });
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
        let mut spawned_count = 0usize;
        for worker_id in 0..GIF_DECODE_WORKERS {
            let rx = job_receiver.clone();
            match std::thread::Builder::new()
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
                }) {
                Ok(_) => spawned_count += 1,
                Err(e) => {
                    log::error!(
                        "[GifManager] Failed to spawn gif-decode-{} worker: {}",
                        worker_id,
                        e
                    );
                }
            }
        }
        if spawned_count == 0 {
            log::error!("[GifManager] No GIF decode workers spawned; GIF playback will not work");
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
                if d.frames.iter().any(|frame| frame.rgba.is_none()) {
                    d.cancelled.store(true, Ordering::SeqCst);
                    let remaining_staging_bytes = d.total_bytes;
                    drop(d);
                    self.cache.pop(path);
                    atomic_saturating_sub(&self.running_total_bytes, remaining_staging_bytes);
                } else {
                    return d_clone;
                }
            }
        }

        // Cleanup before adding new
        self.cleanup(false);

        let generation = self.current_generation.fetch_add(1, Ordering::SeqCst);
        let running_total = self.running_total_bytes.clone();
        let data = Arc::new(Mutex::new(GifData::new(generation, running_total.clone())));
        self.cache.put(path.to_path_buf(), data.clone());

        let path_buf = path.to_path_buf();
        let data_clone = data.clone();
        let current_gen = self.current_generation.clone();
        let ui_ctx = self.ui_ctx.clone();
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
            max_gif_bytes: max_memory_bytes,
        });

        data
    }

    /// Drop every cached GIF except the currently visible preview.
    pub fn unload_except(&mut self, keep_path: Option<&Path>) {
        let keep_path = keep_path.map(Path::to_path_buf);
        let mut total_removed = 0usize;
        let paths_to_remove: Vec<PathBuf> = self
            .cache
            .iter()
            .filter_map(|(path, _)| {
                if keep_path.as_ref().is_some_and(|keep| keep == path) {
                    None
                } else {
                    Some(path.clone())
                }
            })
            .collect();

        for path in paths_to_remove {
            if let Some(data) = self.cache.pop(&path) {
                let d = data.lock();
                d.cancelled.store(true, Ordering::SeqCst);
                total_removed = total_removed.saturating_add(d.total_bytes);
            }
        }

        if total_removed > 0 {
            atomic_saturating_sub(&self.running_total_bytes, total_removed);
        }
    }

    pub fn unload_all(&mut self) {
        self.unload_except(None);
    }

    pub fn stats(&self) -> (usize, usize) {
        (
            self.cache.len(),
            self.running_total_bytes.load(Ordering::SeqCst),
        )
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
            if Arc::strong_count(data) > 1 {
                continue;
            }
            let d = data.lock();
            if now.duration_since(d.last_used) > ttl {
                d.cancelled.store(true, Ordering::SeqCst);
                to_remove.push((path.clone(), d.total_bytes));
            }
        }
        for (path, bytes) in to_remove {
            self.cache.pop(&path);
            atomic_saturating_sub(&self.running_total_bytes, bytes);
        }

        // 2. Memory-based LRU Cleanup - O(1) check using running total
        while self.running_total_bytes.load(Ordering::SeqCst) > self.max_memory_bytes
            && !self.cache.is_empty()
        {
            let mut eviction_candidates = Vec::new();
            for (path, data) in self.cache.iter() {
                if Arc::strong_count(data) > 1 {
                    continue;
                }

                let d = data.lock();
                eviction_candidates.push((path.clone(), d.last_used));
            }

            if eviction_candidates.is_empty() {
                break;
            }

            eviction_candidates.sort_by_key(|(_, last_used)| *last_used);
            let mut evicted_any = false;

            for (path, _) in eviction_candidates {
                if self.running_total_bytes.load(Ordering::SeqCst) <= self.max_memory_bytes {
                    break;
                }

                if let Some(data) = self.cache.pop(&path) {
                    let d = data.lock();
                    d.cancelled.store(true, Ordering::SeqCst);
                    atomic_saturating_sub(&self.running_total_bytes, d.total_bytes);
                    evicted_any = true;
                }
            }

            if !evicted_any {
                break;
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

            // Sidebar previews are at most 240px high, so keeping 512px RGBA
            // frames in RAM is wasteful for animated GIFs.
            let (w, h, rgba) = if orig_w > GIF_PREVIEW_MAX_DIMENSION
                || orig_h > GIF_PREVIEW_MAX_DIMENSION
            {
                let img = image::DynamicImage::ImageRgba8(buffer);
                let resized = img.thumbnail(GIF_PREVIEW_MAX_DIMENSION, GIF_PREVIEW_MAX_DIMENSION);
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
                let exceeds_per_gif_budget =
                    d.retained_frame_bytes.saturating_add(frame_bytes) > max_gif_bytes;
                let exceeds_global_budget =
                    running_total_before.saturating_add(frame_bytes) > max_memory_bytes;

                if has_visible_frames && (exceeds_per_gif_budget || exceeds_global_budget) {
                    d.is_complete = true;
                    ui_ctx.request_repaint();
                    log::debug!(
                        "[GIF] Truncated decode for {:?}: frames={} total_bytes={} next_frame={} global_total={} exceeds_per_gif={} exceeds_global={}",
                        path,
                        d.frames.len(),
                        d.retained_frame_bytes,
                        frame_bytes,
                        running_total_before,
                        exceeds_per_gif_budget,
                        exceeds_global_budget
                    );
                    return Ok(());
                }

                d.total_bytes += frame_bytes;
                d.retained_frame_bytes += frame_bytes;
                // PERFORMANCE: Update running total atomically for O(1) memory tracking
                running_total.fetch_add(frame_bytes, Ordering::SeqCst);
                d.frames.push(DecodedFrame {
                    rgba: Some(rgba),
                    width: w,
                    height: h,
                    original_width: orig_w,
                    original_height: orig_h,
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
