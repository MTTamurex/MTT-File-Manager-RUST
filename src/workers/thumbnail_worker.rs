//! Thumbnail worker for parallel hybrid thumbnail extraction
//! Pipeline: 1. image crate (Fast) -> 2. WIC (Robust/CMYK) -> 3. Shell API (Universal/Video)
//!
//! PERFORMANCE: Uses I/O priority system to:
//! - Minimize disk seeks on HDDs by grouping requests by directory
//! - Adjust thread priority based on request urgency
//! - Prioritize visible thumbnails over prefetch/background work

use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::windows::file_type::is_video_extension;
use eframe::egui;
use image::{DynamicImage, ImageFormat};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Instant, SystemTime};
use windows::core::Interface;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Maximum concurrent decode operations (RAM limiter)
const MAX_CONCURRENT_DECODES: usize = 5;

/// Global cache of paths that failed thumbnail extraction (shared across workers)
/// Prevents re-attempting extraction on files that consistently fail (e.g., corrupt files)
static FAILED_PATHS: std::sync::OnceLock<Mutex<FxHashSet<PathBuf>>> = std::sync::OnceLock::new();

fn get_failed_paths() -> &'static Mutex<FxHashSet<PathBuf>> {
    FAILED_PATHS.get_or_init(|| Mutex::new(FxHashSet::default()))
}

/// Check if a path has previously failed extraction
fn is_known_failure(path: &PathBuf) -> bool {
    get_failed_paths()
        .lock()
        .map(|set| set.contains(path))
        .unwrap_or(false)
}

/// Mark a path as failed (won't retry until app restart)
fn mark_as_failed(path: PathBuf) {
    if let Ok(mut set) = get_failed_paths().lock() {
        // Limit cache size to prevent memory issues (keep last 1000 failures)
        if set.len() > 1000 {
            set.clear();
        }
        set.insert(path);
    }
}

/// Clear failure status for a specific path (allows retry)
/// Used when manually refreshing a thumbnail after file changes
pub fn clear_failure_cache(path: &PathBuf) {
    if let Ok(mut set) = get_failed_paths().lock() {
        set.remove(path);
    }
}

/// Clear all failure status (allows retry for everything)
/// Used when manually refreshing the entire folder (F5)
pub fn clear_all_failures() {
    if let Ok(mut set) = get_failed_paths().lock() {
        set.clear();
    }
}

/// Legacy alias for backwards compatibility with old ThumbnailPriority enum
/// High -> Interactive, Low -> Prefetch
pub type ThumbnailPriority = IOPriority;

/// Thumbnail request with priority and metadata
#[derive(Debug, Clone)]
struct ThumbnailRequest {
    path: PathBuf,
    generation: usize,
    size: u32,
    priority: IOPriority,
    directory_index: Option<usize>,
}

/// Queue state with directory-grouped requests for HDD optimization
struct QueueState {
    /// Requests grouped by parent directory (for HDD locality optimization)
    by_directory: FxHashMap<PathBuf, Vec<ThumbnailRequest>>,

    /// Quick lookup to prevent duplicates
    pending: FxHashSet<PathBuf>,

    /// Whether we're on an SSD (detected on first request)
    is_ssd: Option<bool>,

    /// Current directory being processed (for HDD locality)
    current_directory: Option<PathBuf>,

    /// Shutdown flag
    shutdown: bool,
}

pub struct PriorityThumbnailQueue {
    state: Mutex<QueueState>,
    condvar: Condvar,
}

impl Default for PriorityThumbnailQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl PriorityThumbnailQueue {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                by_directory: FxHashMap::default(),
                pending: FxHashSet::default(),
                is_ssd: None,
                current_directory: None,
                shutdown: false,
            }),
            condvar: Condvar::new(),
        }
    }

    pub fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.shutdown = true;
        self.condvar.notify_all();
    }

    /// Push a thumbnail request with the new IOPriority system
    pub fn push(&self, path: PathBuf, gen: usize, request_size: u32, priority: IOPriority) {
        self.push_with_index(path, gen, request_size, priority, None);
    }

    pub fn push_with_index(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        directory_index: Option<usize>,
    ) {
        let mut state = self.state.lock().unwrap();

        // Deduplication: if already pending, skip
        if !state.pending.insert(path.clone()) {
            return;
        }

        // Detect disk type on first request
        if state.is_ssd.is_none() {
            state.is_ssd = Some(io_priority::is_ssd(&path));
            if !state.is_ssd.unwrap() {
                eprintln!("[IO] HDD detected - enabling directory grouping for seek optimization");
            }
        }

        // Group by parent directory (for HDD seek optimization)
        let parent = path.parent().unwrap_or(&path).to_path_buf();

        let request = ThumbnailRequest {
            path,
            generation: gen,
            size: request_size,
            priority,
            directory_index,
        };

        state.by_directory.entry(parent.clone()).or_default().push(request);

        if !state.is_ssd.unwrap_or(true) {
            if let Some(items) = state.by_directory.get_mut(&parent) {
                items.sort_by(|a, b| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => a.directory_index.cmp(&b.directory_index),
                    other => other,
                });
            }
        }

        self.condvar.notify_one();
    }

    /// Pop the next request, optimizing for disk locality on HDDs
    pub fn pop(&self) -> Option<(PathBuf, usize, u32, IOPriority)> {
        let mut state = self.state.lock().unwrap();

        loop {
            if state.shutdown {
                return None;
            }

            // Try to get next item
            if let Some(request) = Self::pop_next_request(&mut state) {
                state.pending.remove(&request.path);

                // Adjust thread priority based on request priority
                io_priority::set_thread_priority(request.priority);

                return Some((
                    request.path,
                    request.generation,
                    request.size,
                    request.priority,
                ));
            }

            // Wait for new work
            state = self.condvar.wait(state).unwrap();
        }
    }

    /// Get the next request, using locality optimization for HDDs
    fn pop_next_request(state: &mut QueueState) -> Option<ThumbnailRequest> {
        if state.by_directory.is_empty() {
            return None;
        }

        let is_ssd = state.is_ssd.unwrap_or(true);

        if is_ssd {
            // SSD: Just get highest priority item from any directory
            Self::pop_highest_priority(state)
        } else {
            // HDD: Prefer items from current directory to minimize seeks
            Self::pop_with_locality(state)
        }
    }

    /// Pop highest priority item regardless of directory (SSD mode)
    fn pop_highest_priority(state: &mut QueueState) -> Option<ThumbnailRequest> {
        // Find directory with highest priority item
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| {
                items
                    .iter()
                    .map(|r| r.priority)
                    .min()
                    .unwrap_or(IOPriority::Background)
            })
            .map(|(dir, _)| dir.clone())?;

        Self::pop_from_directory(state, &best_dir)
    }

    /// Pop item with locality preference (HDD mode)
    fn pop_with_locality(state: &mut QueueState) -> Option<ThumbnailRequest> {
        // If we have a current directory with items, continue there
        // (unless there's a higher priority item elsewhere)
        if let Some(ref dir) = state.current_directory.clone() {
            if let Some(items) = state.by_directory.get(dir) {
                if !items.is_empty() {
                    // Check if current dir has interactive priority
                    let current_best = items
                        .iter()
                        .map(|r| r.priority)
                        .min()
                        .unwrap_or(IOPriority::Background);

                    // Only switch directories if there's an Interactive request elsewhere
                    let should_switch =
                        state.by_directory.iter().any(|(other_dir, other_items)| {
                            other_dir != dir
                                && other_items
                                    .iter()
                                    .any(|r| r.priority == IOPriority::Interactive)
                                && current_best != IOPriority::Interactive
                        });

                    if !should_switch {
                        return Self::pop_from_directory(state, dir);
                    }
                }
            }
        }

        // Find directory with highest priority item
        let best_dir = state
            .by_directory
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .min_by_key(|(_, items)| {
                items
                    .iter()
                    .map(|r| r.priority)
                    .min()
                    .unwrap_or(IOPriority::Background)
            })
            .map(|(dir, _)| dir.clone())?;

        state.current_directory = Some(best_dir.clone());
        Self::pop_from_directory(state, &best_dir)
    }

    /// Pop highest priority item from a specific directory
    fn pop_from_directory(state: &mut QueueState, dir: &PathBuf) -> Option<ThumbnailRequest> {
        let items = state.by_directory.get_mut(dir)?;

        if items.is_empty() {
            state.by_directory.remove(dir);
            return None;
        }

        let is_ssd = state.is_ssd.unwrap_or(true);
        let best_idx = if is_ssd {
            items
                .iter()
                .enumerate()
                .min_by(|(idx_a, a), (idx_b, b)| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => idx_b.cmp(idx_a),
                    other => other,
                })
                .map(|(idx, _)| idx)?
        } else {
            items
                .iter()
                .enumerate()
                .min_by(|(idx_a, a), (idx_b, b)| match a.priority.cmp(&b.priority) {
                    std::cmp::Ordering::Equal => {
                        let a_index = a.directory_index.unwrap_or(usize::MAX);
                        let b_index = b.directory_index.unwrap_or(usize::MAX);
                        match a_index.cmp(&b_index) {
                            std::cmp::Ordering::Equal => idx_b.cmp(idx_a),
                            other => other,
                        }
                    }
                    other => other,
                })
                .map(|(idx, _)| idx)?
        };

        let request = items.swap_remove(best_idx);

        // Clean up empty directories
        if items.is_empty() {
            state.by_directory.remove(dir);
            if state.current_directory.as_ref() == Some(dir) {
                state.current_directory = None;
            }
        }

        Some(request)
    }
}

/// Semaphore to limit concurrent resource usage
struct Semaphore {
    count: Mutex<usize>,
    condvar: Condvar,
    max: usize,
}

impl Semaphore {
    fn new(max: usize) -> Self {
        Self {
            count: Mutex::new(0),
            condvar: Condvar::new(),
            max,
        }
    }

    fn acquire(&self) {
        let mut count = self.count.lock().unwrap();
        while *count >= self.max {
            count = self.condvar.wait(count).unwrap();
        }
        *count += 1;
    }

    fn release(&self) {
        let mut count = self.count.lock().unwrap();
        if *count > 0 {
            *count -= 1;
        }
        self.condvar.notify_one();
    }
}

/// Spawns thumbnail worker threads with concurrency limiting
pub fn spawn_thumbnail_workers(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
) {
    // Semaphore for RAM limiter
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_DECODES));

    // 4 worker threads
    for _ in 0..4 {
        let queue = queue.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        let disk_cache = disk_cache.clone();
        let semaphore = semaphore.clone();

        std::thread::spawn(move || {
            thumbnail_worker_loop(queue, tx, ctx, gen_tracker, disk_cache, semaphore);
        });
    }
}

fn thumbnail_worker_loop(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
    semaphore: Arc<Semaphore>,
) {
    let mut last_repaint = Instant::now();
    unsafe {
        // SAFETY: Initializing COM with Multithreaded support for this worker thread.
        // It is paired with `CoUninitialize` at the end of the thread loop.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        // SAFETY: Initialize Media Foundation ONCE per thread working with video processing.
        // This avoids the expensive overhead of MFStartup/MFShutdown for every single file.
        // MF_VERSION = 0x00020070, MFSTARTUP_NOSOCKET = 0x1
        use windows::Win32::Media::MediaFoundation::{MFStartup, MFSTARTUP_NOSOCKET};
        if let Err(e) = MFStartup(0x00020070, MFSTARTUP_NOSOCKET) {
            eprintln!(
                "[ThumbnailWorker] Failed to initialize Media Foundation: {:?}",
                e
            );
        } else {
            // Register a deferred shutdown
            // Ideally we would do this in a scope guard, but for this loop structure:
            // We consciously call MFShutdown before CoUninitialize below.
        }
    }

    // PERFORMANCE: Set background priority to minimize HDD contention with video playback
    // This applies to all 4 thumbnail worker threads
    io_priority::set_thread_priority(IOPriority::Background);

    while let Some((path, req_gen, req_size, req_priority)) = queue.pop() {
        // ... loop content ...
        {
            // Block to scope the work variable was unused, flattened loop logic instead
            if true {
                // Simplified matching
                {
                    if req_gen == gen_tracker.load(Ordering::Relaxed) {
                        // ... existing logic ...
                        // The body of the loop remains unchanged except for removing MFStartup/Shutdown calls
                        // inside `try_media_foundation_extraction`.

                        // EARLY EXIT 1: Skip files that already failed in this session
                        // Prevents repeated slow retries on broken files (e.g., 0x8004B205)
                        if is_known_failure(&path) {
                            let _ = tx.send(ThumbnailData {
                                path,
                                image_data: Vec::new(),
                                width: 0,
                                height: 0,
                                generation: req_gen,
                            });
                            throttle_repaint(&ctx, &mut last_repaint);
                            continue;
                        }

                        // EARLY EXIT 1: Validate path exists before processing
                        if !path.exists() {
                            mark_as_failed(path.clone());
                            let _ = tx.send(ThumbnailData {
                                path,
                                image_data: Vec::new(),
                                width: 0,
                                height: 0,
                                generation: req_gen,
                            });
                            throttle_repaint(&ctx, &mut last_repaint);
                            continue;
                        }

                        // EARLY EXIT 2: Skip cloud-only OneDrive files (not downloaded)
                        if crate::infrastructure::onedrive::is_onedrive_path(&path)
                            && !crate::infrastructure::onedrive::is_locally_available(&path)
                        {
                            mark_as_failed(path.clone());
                            let _ = tx.send(ThumbnailData {
                                path,
                                image_data: Vec::new(),
                                width: 0,
                                height: 0,
                                generation: req_gen,
                            });
                            throttle_repaint(&ctx, &mut last_repaint);
                            continue;
                        }

                        let modified = std::fs::metadata(&path)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);

                        let mut final_result = None;

                        // STEP 0: Check Disk Cache with SIZE VALIDATION
                        if let Some((cached_bytes, cached_w, cached_h)) =
                            disk_cache.get(&path, modified)
                        {
                            let cached_max_dim = cached_w.max(cached_h);

                            // Only use cache if it meets or exceeds the requested size
                            // OR if dimensions are unknown (0) from old cache entries - regenerate those
                            if cached_max_dim >= req_size && cached_max_dim > 0 {
                                // Cache is good enough (or better), use it
                                if let Ok(img) = image::load_from_memory_with_format(
                                    &cached_bytes,
                                    ImageFormat::WebP,
                                ) {
                                    let rgba = img.to_rgba8();
                                    final_result =
                                        Some((rgba.to_vec(), rgba.width(), rgba.height()));
                                }
                            } else {
                            }
                            // If cached_max_dim < req_size or == 0, fall through to regeneration
                        } else {
                        }

                        // STEP 1: Se não está em cache, decodifica com limite de concorrência
                        if final_result.is_none() {
                            // Aguarda até ter um slot disponível (max 4 decodes simultâneos)
                            semaphore.acquire();

                            // HYBRID PIPELINE com resize imediato
                            if let Some((raw_data, w, h)) =
                                generate_thumbnail_hybrid(&path, req_priority)
                            {
                                // STEP 2: Resize to bucket (libera RAM e otimiza upload GPU)
                                let bucket_size = get_bucket_size(req_size);
                                let resized = resize_to_bucket(raw_data, w, h, bucket_size);

                                // STEP 3: Salva versão otimizada em SQLite
                                let _ = disk_cache
                                    .put(&path, modified, &resized.0, resized.1, resized.2);

                                // STEP 4: Usa a versão resizada (já otimizada)
                                final_result = Some(resized);
                            } else {
                                // EXTRACTION FAILED: Mark as failed to skip future attempts
                                mark_as_failed(path.clone());
                            }
                            // raw_data é dropado aqui automaticamente (libera RAM)

                            // Libera slot
                            semaphore.release();
                        }

                        let (data, w, h) = final_result.unwrap_or_else(|| (Vec::new(), 0, 0));

                        let _ = tx.send(ThumbnailData {
                            path,
                            image_data: data,
                            width: w,
                            height: h,
                            generation: req_gen,
                        });
                        throttle_repaint(&ctx, &mut last_repaint);
                    }
                }
            }
        }
    }
    unsafe {
        // SAFETY: Cleaning up COM for this thread before exit.
        use windows::Win32::Media::MediaFoundation::MFShutdown;
        let _ = MFShutdown();
        CoUninitialize();
    }
}

fn get_bucket_size(req_size: u32) -> u32 {
    match req_size {
        0..=128 => 128,
        129..=256 => 256,
        257..=512 => 512,
        _ => 1024,
    }
}

/// Resize RGBA buffer to bucket size while preserving aspect ratio
fn resize_to_bucket(
    rgba_data: Vec<u8>,
    width: u32,
    height: u32,
    max_dim: u32,
) -> (Vec<u8>, u32, u32) {
    // Se já é pequeno o suficiente, retorna como está
    if width <= max_dim && height <= max_dim {
        return (rgba_data, width, height);
    }

    // Calcula novo tamanho mantendo aspect ratio
    let scale = (max_dim as f32) / (width.max(height) as f32);
    let new_w = ((width as f32) * scale).round() as u32;
    let new_h = ((height as f32) * scale).round() as u32;

    // Ensure we don't lose the buffer if from_raw fails (which consumes it)
    // We check the condition beforehand.
    // ImageBuffer::from_raw requires buffer.len() >= width * height * 4
    let min_len = (width as usize) * (height as usize) * 4;

    if rgba_data.len() >= min_len {
        // Usa image crate para resize
        // Safe to unwrap because we checked the dimensions
        let img = image::ImageBuffer::from_raw(width, height, rgba_data)
            .expect("Buffer size check passed but from_raw failed");

        let dynamic = DynamicImage::ImageRgba8(img);
        // Use CatmullRom for high-quality sharpening with good performance.
        let resized = dynamic.resize(new_w, new_h, image::imageops::FilterType::CatmullRom);
        let rgba = resized.into_rgba8();
        let w = rgba.width();
        let h = rgba.height();
        return (rgba.into_vec(), w, h);
    }

    // Fallback: retorna original se resize falhar ou tamanho incorreto
    (rgba_data, width, height)
}

/// The 4-Step Hybrid Pipeline
fn generate_thumbnail_hybrid(
    path: &Path,
    priority: IOPriority,
) -> Option<(Vec<u8>, u32, u32)> {
    // Stage 1: image crate (Fast Path)
    if let Some(result) = try_image_crate_extraction(path, priority) {
        return Some(result);
    }

    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    if let Some(result) = try_wic_extraction(path) {
        return Some(result);
    }

    // Stage 3: Shell API (Universal/Video)
    match extract_windows_thumbnail_shell(path) {
        Ok(result) => return Some(result),
        Err(e) => {
            let err_str = e.to_string();
            // Don't log "File Not Found" errors as they are expected for recently deleted files
            if !err_str.contains("0x80070002") {
                eprintln!(
                    "[Thumbnail] Stage 3 failed for {:?}: {}",
                    path.file_name(),
                    e
                );
            }
        }
    }

    // Stage 4: IThumbnailCache with WTS_FORCEEXTRACTION (bypassa cache do Windows)
    // Útil quando o cache do Windows retornou um ícone em vez do thumbnail real
    // Single attempt - if fails, Stage 5 takes over
    match crate::infrastructure::windows::icons::force_extract_thumbnail(path) {
        Ok(result) => return Some(result),
        Err(e) => {
            let err_str = e.to_string();
            // Don't log "File Not Found" errors as they are expected for recently deleted files
            if !err_str.contains("0x80070002") {
                eprintln!(
                    "[Thumbnail] Stage 4 (force) failed for {:?}: {}",
                    path.file_name(),
                    e
                );
            }
        }
    }

    // Stage 5: Media Foundation direct frame extraction (bypasses Windows thumbnail service)
    // This is the nuclear option - extracts a raw video frame when all else fails
    if let Some(result) = try_media_foundation_extraction(path) {
        return Some(result);
    }

    None
}

fn try_image_crate_extraction(
    path: &Path,
    priority: IOPriority,
) -> Option<(Vec<u8>, u32, u32)> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tiff"
    ) {
        return None;
    }

    use crate::infrastructure::windows::file_flags::{
        open_sequential, open_sequential_background, open_sequential_low_priority,
    };
    use std::io::BufReader;

    let file = match priority {
        IOPriority::Interactive => open_sequential(path).ok()?,
        IOPriority::Prefetch => open_sequential_low_priority(path).ok()?,
        IOPriority::Background => open_sequential_background(path).ok()?,
    };
    let reader = BufReader::with_capacity(65536, file);
    let format = ImageFormat::from_extension(&ext)?;

    match image::load(reader, format) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            Some((rgba.to_vec(), rgba.width(), rgba.height()))
        }
        Err(_) => None,
    }
}

fn try_wic_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // WIC is for image files only - videos should go directly to Shell API (Stage 3)
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tiff" | "webp" | "ico" | "tif"
    ) {
        return None;
    }

    use windows::{
        core::PCWSTR, Win32::Foundation::GENERIC_ACCESS_RIGHTS, Win32::Graphics::Imaging::*,
        Win32::System::Com::*,
    };

    unsafe {
        // SAFETY: All WIC components are used within this block and the COM library
        // has been initialized for this thread. Raw pointers from `path_wide` are
        // valid for the duration of the call.
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;

        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(path_wide.as_ptr()),
                None,
                GENERIC_ACCESS_RIGHTS(0x80000000), // GENERIC_READ
                WICDecodeMetadataCacheOnDemand,
            )
            .ok()?;

        let frame = decoder.GetFrame(0).ok()?;

        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppRGBA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .ok()?;

        let mut width = 0;
        let mut height = 0;
        converter.GetSize(&mut width, &mut height).ok()?;

        let mut buffer = vec![0u8; (width * height * 4) as usize];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut buffer)
            .ok()?;

        Some((buffer, width, height))
    }
}

/// Stage 5: Media Foundation direct frame extraction (nuclear option)
///
/// Bypasses the Windows thumbnail service entirely by directly reading
/// a video frame using IMFSourceReader. This works even when the thumbnail
/// cache is broken or returns 0x8004B205 (extraction pending) indefinitely.
fn try_media_foundation_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // Only for video files - use centralized extension check
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !is_video_extension(&ext) {
        return None;
    }

    let mf_start = std::time::Instant::now();
    eprintln!(
        "[Thumbnail] Stage 5 (Media Foundation) attempting: {:?}",
        path.file_name()
    );

    use std::os::windows::ffi::OsStrExt;
    use windows::{core::PCWSTR, Win32::Media::MediaFoundation::*};

    unsafe {
        // MFStartup/Shutdown - the thumbnail worker thread already has COM initialized
        // SAFETY: MF_VERSION = 0x00020070 (MF 2.0)
        // MFStartup is now called ONCE at thread start (see thumbnail_worker_loop)
        // so we don't need to call it here for every file.
        // if MFStartup(0x00020070, MFSTARTUP_NOSOCKET).is_err() { ... }

        // Convert path to wide string
        let wide_path: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // Create source reader
        let reader: IMFSourceReader =
            match MFCreateSourceReaderFromURL(PCWSTR(wide_path.as_ptr()), None) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "[Thumbnail] Stage 5: Failed to create source reader: {:?}",
                        e
                    );
                    // MFShutdown moved to thread lifecycle
                    return None;
                }
            };

        // Get the first video stream's native media type
        let media_type: IMFMediaType = match reader.GetNativeMediaType(
            0xFFFFFFFC, // MF_SOURCE_READER_FIRST_VIDEO_STREAM
            0,
        ) {
            Ok(mt) => mt,
            Err(e) => {
                eprintln!("[Thumbnail] Stage 5: No video stream found: {:?}", e);
                // MFShutdown moved to thread lifecycle
                return None;
            }
        };

        // Get video dimensions
        let frame_size = media_type.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let width = (frame_size >> 32) as u32;
        let height = (frame_size & 0xFFFFFFFF) as u32;

        if width == 0 || height == 0 {
            eprintln!("[Thumbnail] Stage 5: Invalid dimensions");
            // MFShutdown moved to thread lifecycle
            return None;
        }

        // Try RGB32 first, fallback to NV12 if not supported
        let output_type: IMFMediaType = match MFCreateMediaType() {
            Ok(mt) => mt,
            Err(_) => {
                // MFShutdown moved to thread lifecycle
                return None;
            }
        };

        // MFMediaType_Video GUID
        let mf_video_guid = windows::core::GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
        // MFVideoFormat_RGB32 GUID
        let rgb32_guid = windows::core::GUID::from_u128(0x00000016_0000_0010_8000_00aa00389b71);
        // MFVideoFormat_NV12 GUID
        let nv12_guid = windows::core::GUID::from_u128(0x3231564e_0000_0010_8000_00aa00389b71);

        let _ = output_type.SetGUID(&MF_MT_MAJOR_TYPE, &mf_video_guid);
        let _ = output_type.SetGUID(&MF_MT_SUBTYPE, &rgb32_guid);

        // Try RGB32 first
        let use_nv12 = if reader
            .SetCurrentMediaType(0xFFFFFFFC, None, &output_type)
            .is_err()
        {
            eprintln!("[Thumbnail] Stage 5: RGB32 not supported, falling back to NV12");
            // Fallback to NV12 (universally supported by video decoders)
            let _ = output_type.SetGUID(&MF_MT_SUBTYPE, &nv12_guid);
            if reader
                .SetCurrentMediaType(0xFFFFFFFC, None, &output_type)
                .is_err()
            {
                eprintln!("[Thumbnail] Stage 5: Failed to set NV12 output");
                // MFShutdown moved to thread lifecycle
                return None;
            }
            true
        } else {
            false
        };

        // Skip seeking for now - just read the first frame after position 0
        // This avoids complex PROPVARIANT handling

        // Read a video frame
        let mut stream_index: u32 = 0;
        let mut flags: u32 = 0;
        let mut timestamp: i64 = 0;
        let mut sample: Option<IMFSample> = None;

        let result = reader.ReadSample(
            0xFFFFFFFC, // MF_SOURCE_READER_FIRST_VIDEO_STREAM
            0,          // No control flags
            Some(&mut stream_index as *mut u32),
            Some(&mut flags as *mut u32),
            Some(&mut timestamp as *mut i64),
            Some(&mut sample as *mut Option<IMFSample>),
        );

        if result.is_err() {
            eprintln!("[Thumbnail] Stage 5: ReadSample failed: {:?}", result.err());
            // MFShutdown moved to thread lifecycle
            return None;
        }

        let sample = match sample {
            Some(s) => s,
            None => {
                eprintln!("[Thumbnail] Stage 5: No sample returned");
                // MFShutdown moved to thread lifecycle
                return None;
            }
        };

        // Convert sample to buffer
        let buffer = match sample.ConvertToContiguousBuffer() {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "[Thumbnail] Stage 5: ConvertToContiguousBuffer failed: {:?}",
                    e
                );
                // MFShutdown moved to thread lifecycle
                return None;
            }
        };

        let mut data_ptr: *mut u8 = std::ptr::null_mut();
        let mut max_len: u32 = 0;
        let mut current_len: u32 = 0;

        if buffer
            .Lock(&mut data_ptr, Some(&mut max_len), Some(&mut current_len))
            .is_err()
        {
            eprintln!("[Thumbnail] Stage 5: Lock failed");
            // MFShutdown moved to thread lifecycle
            return None;
        }

        // Convert to RGBA based on format
        let rgba_data = if use_nv12 {
            // NV12 format: Y plane (width*height bytes) + UV plane (width*height/2 bytes)
            let y_size = (width * height) as usize;
            let uv_size = y_size / 2;
            let expected_size = y_size + uv_size;

            if (current_len as usize) < expected_size {
                eprintln!(
                    "[Thumbnail] Stage 5: NV12 buffer size mismatch: {} vs expected {}",
                    current_len, expected_size
                );
                let _ = buffer.Unlock();
                // MFShutdown moved to thread lifecycle
                return None;
            }

            let nv12_slice = std::slice::from_raw_parts(data_ptr, expected_size);
            convert_nv12_to_rgba(nv12_slice, width, height)
        } else {
            // RGB32 format: straight BGRA copy and swap
            let expected_size = (width * height * 4) as usize;
            if (current_len as usize) < expected_size {
                eprintln!(
                    "[Thumbnail] Stage 5: RGB32 buffer size mismatch: {} vs expected {}",
                    current_len, expected_size
                );
                let _ = buffer.Unlock();
                // MFShutdown moved to thread lifecycle
                return None;
            }

            let mut rgba_data = vec![0u8; expected_size];
            std::ptr::copy_nonoverlapping(data_ptr, rgba_data.as_mut_ptr(), expected_size);
            rgba_data
        };

        let _ = buffer.Unlock();
        // MFShutdown moved to thread lifecycle

        // Convert BGRA to RGBA if RGB32 was used (swap R and B channels)
        let mut rgba_data = rgba_data;
        if !use_nv12 {
            for pixel in rgba_data.chunks_exact_mut(4) {
                pixel.swap(0, 2); // Swap B and R
            }
        }

        let mf_elapsed = mf_start.elapsed();
        eprintln!(
            "[Thumbnail] Stage 5 SUCCESS: {:?} ({}x{}) in {:.2}s",
            path.file_name(),
            width,
            height,
            mf_elapsed.as_secs_f64()
        );
        Some((rgba_data, width, height))
    }
}

/// Convert NV12 format to RGBA
///
/// NV12 layout:
/// - Y plane: width*height bytes (luminance)
/// - UV plane: width*height/2 bytes (interleaved U,V pairs)
fn convert_nv12_to_rgba(nv12_data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let y_size = width * height;

    let y_plane = &nv12_data[0..y_size];
    let uv_plane = &nv12_data[y_size..];

    let mut rgba = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let y_index = y * width + x;
            let uv_index = (y / 2) * (width / 2) * 2 + (x / 2) * 2;

            let y_val = y_plane[y_index] as i32;
            let u_val = uv_plane[uv_index] as i32 - 128;
            let v_val = uv_plane[uv_index + 1] as i32 - 128;

            // YUV to RGB conversion (BT.601 standard) optimized with integer arithmetic
            // 1.402 * 1024 = 1435.648 -> 1436
            // 0.344 * 1024 = 352.256 -> 352
            // 0.714 * 1024 = 731.136 -> 731
            // 1.772 * 1024 = 1814.528 -> 1815
            let y_shifted = y_val << 10;
            let r = ((y_shifted + 1436 * v_val) >> 10).clamp(0, 255);
            let g = ((y_shifted - 352 * u_val - 731 * v_val) >> 10).clamp(0, 255);
            let b = ((y_shifted + 1815 * u_val) >> 10).clamp(0, 255);

            let rgba_index = y_index * 4;
            rgba[rgba_index] = r as u8;
            rgba[rgba_index + 1] = g as u8;
            rgba[rgba_index + 2] = b as u8;
            rgba[rgba_index + 3] = 255; // Alpha
        }
    }

    rgba
}

fn extract_windows_thumbnail_shell(
    path: &Path,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::{
        core::PCWSTR,
        Win32::Graphics::Gdi::{DeleteObject, HBITMAP},
        Win32::UI::Shell::{
            IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_RESIZETOFIT,
            SIIGBF_THUMBNAILONLY,
        },
    };

    // Determine size based on file type - use centralized extension check
    // Videos: 512px (high quality for preview panel)
    // Others: 1024px (high-res system icons, executables, etc.)
    let is_video = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| is_video_extension(&ext.to_lowercase()))
        .unwrap_or(false);

    let size_px = if is_video { 512 } else { 1024 };

    unsafe {
        // SAFETY: Raw pointers from `path_wide` are valid for the call.
        // HBITMAP is a resource that is manually deleted with `DeleteObject` below.
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;
        let image_factory: IShellItemImageFactory = shell_item.cast()?;

        let size = windows::Win32::Foundation::SIZE {
            cx: size_px,
            cy: size_px,
        };

        // Para vídeos: usa THUMBNAILONLY para FALHAR se só tiver ícone
        // Isso permite que Stage 4 (force extraction) seja acionado
        // Para outros arquivos: usa RESIZETOFIT que aceita ícones
        let flags = if is_video {
            SIIGBF_THUMBNAILONLY
        } else {
            SIIGBF_RESIZETOFIT
        };
        let hbitmap: HBITMAP = image_factory.GetImage(size, flags)?;

        let (rgba_data, width, height) = hbitmap_to_rgba(hbitmap)?;
        let _ = DeleteObject(hbitmap.into());

        Ok((rgba_data, width, height))
    }
}

fn hbitmap_to_rgba(
    hbitmap: windows::Win32::Graphics::Gdi::HBITMAP,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        // SAFETY: `bm` is properly initialized before being passed to `GetObjectW`.
        // `buffer` is pre-allocated with correct size. `hbitmap` is a valid handle.
        let mut bm = BITMAP::default();
        GetObjectW(
            hbitmap.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );

        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.unsigned_abs() as usize;
        let mut buffer = vec![0u8; width * height * 4];

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let hdc = GetDC(None);
        GetDIBits(
            hdc,
            hbitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);

        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        Ok((buffer, width as u32, height as u32))
    }
}

fn throttle_repaint(ctx: &egui::Context, last_repaint: &mut Instant) {
    if last_repaint.elapsed().as_millis() >= 33 {
        ctx.request_repaint();
        *last_repaint = Instant::now();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn test_semaphore_concurrency() {
        let max_concurrent = 2;
        let semaphore = Arc::new(Semaphore::new(max_concurrent));
        let active_count = Arc::new(Mutex::new(0));

        let mut handles = vec![];

        for i in 0..5 {
            let semaphore = semaphore.clone();
            let active_count = active_count.clone();

            handles.push(thread::spawn(move || {
                semaphore.acquire();

                {
                    let mut count = active_count.lock().unwrap();
                    *count += 1;
                    assert!(*count <= max_concurrent, "Too many threads!");
                    println!("Thread {} running. Active: {}", i, *count);
                }

                thread::sleep(Duration::from_millis(50));

                {
                    let mut count = active_count.lock().unwrap();
                    *count -= 1;
                }

                semaphore.release();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_read_coalescing_order_hdd() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("dir");
        std::fs::create_dir(&parent).unwrap();
        let path_a = parent.join("a.png");
        let path_b = parent.join("b.png");

        let queue = PriorityThumbnailQueue::new();
        {
            let mut state = queue.state.lock().unwrap();
            state.is_ssd = Some(false);
        }

        queue.push_with_index(path_a.clone(), 1, 64, IOPriority::Prefetch, Some(2));
        queue.push_with_index(path_b.clone(), 1, 64, IOPriority::Prefetch, Some(1));

        let (path, _, _, _) = queue.pop().unwrap();
        assert_eq!(path, path_b);
    }
}
