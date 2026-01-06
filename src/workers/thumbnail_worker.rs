//! Thumbnail worker for parallel hybrid thumbnail extraction
//! Pipeline: 1. image crate (Fast) -> 2. WIC (Robust/CMYK) -> 3. Shell API (Universal/Video)

use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use eframe::egui;
use image::{DynamicImage, ImageFormat};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use windows::core::Interface;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Maximum concurrent decode operations (RAM limiter)
const MAX_CONCURRENT_DECODES: usize = 4;

/// Spawns thumbnail worker threads with concurrency limiting
pub fn spawn_thumbnail_workers(
    shared_rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
) {
    // AtomicUsize counter to limit concurrent decode operations (RAM limiter)
    let active_decodes = Arc::new(AtomicUsize::new(0));

    // 4 worker threads (reduced from 8 - counter limits actual concurrent work)
    for _ in 0..4 {
        let rx = shared_rx.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        let disk_cache = disk_cache.clone();
        let active_counter = active_decodes.clone();

        std::thread::spawn(move || {
            thumbnail_worker_loop(rx, tx, ctx, gen_tracker, disk_cache, active_counter);
        });
    }
}

fn thumbnail_worker_loop(
    rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
    active_decodes: Arc<AtomicUsize>,
) {
    unsafe {
        // SAFETY: Initializing COM with Multithreaded support for this worker thread.
        // It is paired with `CoUninitialize` at the end of the thread loop.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    loop {
        let work = match rx.lock() {
            Ok(lock) => lock.recv(),
            Err(_) => break,
        };

        match work {
            Ok((path, req_gen)) => {
                if req_gen == gen_tracker.load(Ordering::Relaxed) {
                    let modified = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);

                    let mut final_result = None;

                    // STEP 0: Check Disk Cache (já otimizado, vai direto para UI)
                    if let Some(cached_bytes) = disk_cache.get(&path, modified) {
                        if let Ok(img) =
                            image::load_from_memory_with_format(&cached_bytes, ImageFormat::WebP)
                        {
                            let rgba = img.to_rgba8();
                            final_result = Some((rgba.to_vec(), rgba.width(), rgba.height()));
                        }
                    }

                    // STEP 1: Se não está em cache, decodifica com limite de concorrência
                    if final_result.is_none() {
                        // Aguarda até ter um slot disponível (max 4 decodes simultâneos)
                        while active_decodes.load(Ordering::Relaxed) >= MAX_CONCURRENT_DECODES {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        active_decodes.fetch_add(1, Ordering::Relaxed);

                        // HYBRID PIPELINE com resize imediato
                        if let Some((raw_data, w, h)) = generate_thumbnail_hybrid(&path) {
                            // STEP 2: Resize imediato para 1024px (libera RAM do full-res)
                            let resized = resize_to_max_1024(&raw_data, w, h);

                            // STEP 3: Salva versão otimizada em SQLite
                            let _ =
                                disk_cache.put(&path, modified, &resized.0, resized.1, resized.2);

                            // STEP 4: Usa a versão resizada (já otimizada)
                            final_result = Some(resized);
                        }
                        // raw_data é dropado aqui automaticamente (libera RAM)

                        // Libera slot
                        active_decodes.fetch_sub(1, Ordering::Relaxed);
                    }

                    let (data, w, h) = final_result.unwrap_or_else(|| create_error_placeholder());

                    let _ = tx.send(ThumbnailData {
                        path,
                        image_data: data,
                        width: w,
                        height: h,
                        generation: req_gen,
                    });
                    ctx.request_repaint();
                }
            }
            Err(_) => break,
        }
    }
    unsafe {
        // SAFETY: Cleaning up COM for this thread before exit.
        CoUninitialize();
    }
}

/// Resize RGBA buffer to max 1024x1024 while preserving aspect ratio
fn resize_to_max_1024(rgba_data: &[u8], width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    // Se já é pequeno o suficiente, retorna como está
    if width <= 1024 && height <= 1024 {
        return (rgba_data.to_vec(), width, height);
    }

    // Calcula novo tamanho mantendo aspect ratio
    let scale = 1024.0 / (width.max(height) as f32);
    let new_w = ((width as f32) * scale).round() as u32;
    let new_h = ((height as f32) * scale).round() as u32;

    // Usa image crate para resize
    if let Some(img) = image::ImageBuffer::from_raw(width, height, rgba_data.to_vec()) {
        let dynamic = DynamicImage::ImageRgba8(img);
        let resized = dynamic.resize(new_w, new_h, image::imageops::FilterType::Lanczos3);
        let rgba = resized.to_rgba8();
        return (rgba.to_vec(), rgba.width(), rgba.height());
    }

    // Fallback: retorna original se resize falhar
    (rgba_data.to_vec(), width, height)
}

/// The 4-Step Hybrid Pipeline
fn generate_thumbnail_hybrid(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // Stage 1: image crate (Fast Path)
    if let Some(result) = try_image_crate_extraction(path) {
        return Some(result);
    }

    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    if let Some(result) = try_wic_extraction(path) {
        return Some(result);
    }

    // Stage 3: Shell API (Universal/Video)
    match extract_windows_thumbnail_shell(path) {
        Ok(result) => return Some(result),
        Err(e) => eprintln!("[Thumbnail] Stage 3 failed for {:?}: {}", path.file_name(), e),
    }

    // Stage 4: IThumbnailCache with WTS_FORCEEXTRACTION (bypassa cache do Windows)
    // Útil quando o cache do Windows retornou um ícone em vez do thumbnail real
    match crate::infrastructure::windows::icons::force_extract_thumbnail(path) {
        Ok(result) => return Some(result),
        Err(e) => eprintln!("[Thumbnail] Stage 4 (force) failed for {:?}: {}", path.file_name(), e),
    }
    
    None
}

fn try_image_crate_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tiff"
    ) {
        return None;
    }

    match image::open(path) {
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

fn extract_windows_thumbnail_shell(
    path: &Path,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::{
        core::PCWSTR,
        Win32::Graphics::Gdi::{DeleteObject, HBITMAP},
        Win32::UI::Shell::{
            IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, 
            SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
        },
    };

    // Determine size based on file type
    // Videos: 512px (high quality for preview panel)
    // Others: 1024px (high-res system icons, executables, etc.)
    let is_video = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let ext_lower = ext.to_lowercase();
            matches!(
                ext_lower.as_str(),
                "mp4"
                    | "mkv"
                    | "avi"
                    | "mov"
                    | "wmv"
                    | "flv"
                    | "webm"
                    | "m4v"
                    | "mpg"
                    | "mpeg"
                    | "3gp"
                    | "3g2"
                    | "ts"
                    | "mts"
                    | "m2ts"
                    | "vob"
                    | "ogv"
                    | "divx"
                    | "f4v"
                    | "rm"
                    | "rmvb"
                    | "asf"
            )
        })
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
        let flags = if is_video { SIIGBF_THUMBNAILONLY } else { SIIGBF_RESIZETOFIT };
        let hbitmap: HBITMAP = image_factory.GetImage(size, flags)?;

        let (rgba_data, width, height) = hbitmap_to_rgba(hbitmap)?;
        let _ = DeleteObject(hbitmap);

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
            hbitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );

        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.abs() as usize;
        let mut buffer = vec![0u8; width * height * 4];

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
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

fn create_error_placeholder() -> (Vec<u8>, u32, u32) {
    let size = 512; // Match HiDPI generation size
    let mut buffer = vec![0u8; size * size * 4];
    for (i, pixel) in buffer.chunks_exact_mut(4).enumerate() {
        let x = i % size;
        let y = i / size;
        let intensity = ((x + y) as f32 / (size * 2) as f32 * 100.0) as u8 + 100;
        pixel[0] = intensity;
        pixel[1] = intensity;
        pixel[2] = intensity;
        pixel[3] = 255;
    }
    (buffer, 512, 512)
}
