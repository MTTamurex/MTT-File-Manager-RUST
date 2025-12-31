//! Thumbnail worker for parallel thumbnail extraction
//! Follows .cursorrules: I/O in worker threads, zero allocations in hot path

use std::path::PathBuf;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::domain::thumbnail::ThumbnailData;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED, CoUninitialize};
use eframe::egui;
use windows::core::Interface;

/// Spawns thumbnail worker threads
pub fn spawn_thumbnail_workers(
    shared_rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
) {
    // 8 threads: otimizado para SSDs e pastas grandes
    for _ in 0..8 {
        let rx = shared_rx.clone();
        let tx = tx.clone();
        let gen_tracker = gen_tracker.clone();
        let ctx = ctx.clone();
        
        std::thread::spawn(move || {
            thumbnail_worker_loop(rx, tx, ctx, gen_tracker);
        });
    }
}

/// Main worker loop for thumbnail extraction
fn thumbnail_worker_loop(
    rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
) {
    // Initialize COM for this thread (required for Windows Shell APIs)
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    
    loop {
        let work = {
            match rx.lock() {
                Ok(lock) => lock.recv(),
                Err(_) => break, // App closed
            }
        };
        
        match work {
            Ok((path, req_gen)) => {
                // FAST CANCEL: If global generation changed, ignore before disk read
                if req_gen == gen_tracker.load(Ordering::Relaxed) {
                    // RETRY MECHANISM: Try up to 3 times for transient failures
                    let mut result = None;
                    for attempt in 0..3 {
                        match extract_windows_thumbnail(&path) {
                            Ok((data, w, h)) => {
                                result = Some((data, w, h));
                                break;
                            }
                            Err(_) => {
                                if attempt < 2 {
                                    // Small delay before retry (Windows Shell cache may need time)
                                    std::thread::sleep(std::time::Duration::from_millis(50));
                                }
                            }
                        }
                    }
                    
                    // Only send if extraction succeeded or after all retries
                    let (data, w, h) = result.unwrap_or_else(|| create_error_placeholder());
                    
                    let _ = tx.send(ThumbnailData {
                        path,
                        image_data: data,
                        width: w,
                        height: h,
                        generation: req_gen,
                    });
                    
                    // WAKE UI: Inform that a new thumbnail is ready
                    ctx.request_repaint();
                }
            }
            Err(_) => break,
        }
    }
    
    unsafe { CoUninitialize(); }
}

/// Extracts thumbnail using Windows Shell API
fn extract_windows_thumbnail(path: &PathBuf) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::{
        Win32::UI::Shell::{SHCreateItemFromParsingName, IShellItem, IShellItemImageFactory, SIIGBF_RESIZETOFIT},
        Win32::Graphics::Gdi::{DeleteObject, HBITMAP},
        core::PCWSTR,
    };
    
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let shell_item: IShellItem = SHCreateItemFromParsingName(
            PCWSTR(path_wide.as_ptr()),
            None,
        )?;
        
        let image_factory: IShellItemImageFactory = shell_item.cast()?;
        
        let size = windows::Win32::Foundation::SIZE {
            cx: 256,
            cy: 256,
        };
        // SIIGBF_RESIZETOFIT forces thumbnail generation (even for videos without cached thumbnails)
        let hbitmap: HBITMAP = image_factory.GetImage(size, SIIGBF_RESIZETOFIT)?;
        
        let (rgba_data, width, height) = hbitmap_to_rgba(hbitmap)?;
        
        let _ = DeleteObject(hbitmap);
        
        Ok((rgba_data, width, height))
    }
}

/// Converts HBITMAP to RGBA buffer
fn hbitmap_to_rgba(hbitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    use windows::{
        Win32::Graphics::Gdi::*,
        Win32::Graphics::Gdi::{GetObjectW, GetDC, ReleaseDC, GetDIBits, DIB_RGB_COLORS},
    };
    
    unsafe {
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
        
        // BGRA → RGBA conversion
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

/// Creates error placeholder thumbnail
fn create_error_placeholder() -> (Vec<u8>, u32, u32) {
    let size = 256;
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
    
    (buffer, 256, 256)
}
