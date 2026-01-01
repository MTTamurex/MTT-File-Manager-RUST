//! Thumbnail worker for parallel hybrid thumbnail extraction
//! Pipeline: 1. image crate (Fast) -> 2. WIC (Robust/CMYK) -> 3. Shell API (Universal/Video)

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, Receiver};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::domain::thumbnail::ThumbnailData;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED, CoUninitialize};
use eframe::egui;
use windows::core::Interface;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use std::time::SystemTime;
use image::ImageFormat;

/// Spawns thumbnail worker threads
pub fn spawn_thumbnail_workers(
    shared_rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
) {
    for _ in 0..8 {
        let rx = shared_rx.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        let disk_cache = disk_cache.clone();
        
        std::thread::spawn(move || {
            thumbnail_worker_loop(rx, tx, ctx, gen_tracker, disk_cache);
        });
    }
}

fn thumbnail_worker_loop(
    rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
) {
    unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
    
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
                    
                    // STEP 0: Check Disk Cache
                    if let Some(cached_bytes) = disk_cache.get(&path, modified) {
                        if let Ok(img) = image::load_from_memory_with_format(&cached_bytes, ImageFormat::WebP) {
                            let rgba = img.to_rgba8();
                            final_result = Some((rgba.to_vec(), rgba.width(), rgba.height()));
                        }
                    }
                    
                    if final_result.is_none() {
                        // HYBRID PIPELINE
                        final_result = generate_thumbnail_hybrid(&path);
                        
                        if let Some((ref data, w, h)) = final_result {
                            let _ = disk_cache.put(&path, modified, data, w, h);
                        }
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
    unsafe { CoUninitialize(); }
}

/// The 3-Step Hybrid Pipeline
fn generate_thumbnail_hybrid(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // Stage 1: image crate (Fast Path)
    if let Some(result) = try_image_crate_extraction(path) {
        return Some(result);
    }
    
    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    if let Some(result) = try_wic_extraction(path) {
        return Some(result);
    }
    
    // Stage 3: Shell API (Universal/Video Fallback)
    extract_windows_thumbnail_shell(path).ok()
}

fn try_image_crate_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tiff") {
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
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "bmp" | "gif" | "tiff" | "webp" | "ico" | "tif") {
        return None;
    }

    use windows::{
        core::PCWSTR,
        Win32::System::Com::*,
        Win32::Graphics::Imaging::*,
        Win32::Foundation::GENERIC_ACCESS_RIGHTS,
    };

    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR(path_wide.as_ptr()),
            None,
            GENERIC_ACCESS_RIGHTS(0x80000000), // GENERIC_READ
            WICDecodeMetadataCacheOnDemand,
        ).ok()?;
        
        let frame = decoder.GetFrame(0).ok()?;
        
        let converter = factory.CreateFormatConverter().ok()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        ).ok()?;
        
        let mut width = 0;
        let mut height = 0;
        converter.GetSize(&mut width, &mut height).ok()?;
        
        let mut buffer = vec![0u8; (width * height * 4) as usize];
        converter.CopyPixels(
            std::ptr::null(),
            width * 4,
            &mut buffer,
        ).ok()?;
        
        Some((buffer, width, height))
    }
}

fn extract_windows_thumbnail_shell(path: &Path) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::{
        Win32::UI::Shell::{SHCreateItemFromParsingName, IShellItem, IShellItemImageFactory, SIIGBF_RESIZETOFIT},
        Win32::Graphics::Gdi::{DeleteObject, HBITMAP},
        core::PCWSTR,
    };
    
    // Determine size based on file type
    // Videos: 256px (better compatibility, faster extraction, less memory)
    // Others: 512px (system icons, executables, etc.)
    let is_video = path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            let ext_lower = ext.to_lowercase();
            matches!(ext_lower.as_str(), 
                "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | 
                "mpg" | "mpeg" | "3gp" | "3g2" | "ts" | "mts" | "m2ts" | "vob" | 
                "ogv" | "divx" | "f4v" | "rm" | "rmvb" | "asf")
        })
        .unwrap_or(false);
    
    let size_px = if is_video { 256 } else { 512 };
    
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let shell_item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path_wide.as_ptr()), None)?;
        let image_factory: IShellItemImageFactory = shell_item.cast()?;
        
        let size = windows::Win32::Foundation::SIZE { cx: size_px, cy: size_px };
        let hbitmap: HBITMAP = image_factory.GetImage(size, SIIGBF_RESIZETOFIT)?;
        
        let (rgba_data, width, height) = hbitmap_to_rgba(hbitmap)?;
        let _ = DeleteObject(hbitmap);
        
        Ok((rgba_data, width, height))
    }
}

fn hbitmap_to_rgba(hbitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(hbitmap, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        
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
        GetDIBits(hdc, hbitmap, 0, height as u32, Some(buffer.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

fn create_error_placeholder() -> (Vec<u8>, u32, u32) {
    let size = 512;  // Match HiDPI generation size
    let mut buffer = vec![0u8; size * size * 4];
    for (i, pixel) in buffer.chunks_exact_mut(4).enumerate() {
        let x = i % size;
        let y = i / size;
        let intensity = ((x + y) as f32 / (size * 2) as f32 * 100.0) as u8 + 100;
        pixel[0] = intensity; pixel[1] = intensity; pixel[2] = intensity; pixel[3] = 255;
    }
    (buffer, 512, 512)
}
