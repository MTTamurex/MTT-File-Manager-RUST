//! Taskbar minimize handling for the borderless main window.
//!
//! The wgpu surface can be unavailable to DWM after minimize, producing a black
//! taskbar preview. For minimize we cache a real screenshot of the current frame
//! and provide that bitmap to DWM while the window is minimized.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Mutex;

use eframe::egui;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetIconicLivePreviewBitmap, DwmSetIconicThumbnail, DwmSetWindowAttribute,
    DWMWA_HAS_ICONIC_BITMAP,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, PostMessageW, ShowWindow, SW_MINIMIZE, WM_APP,
    WM_DWMSENDICONICLIVEPREVIEWBITMAP, WM_DWMSENDICONICTHUMBNAIL, WM_NULL,
};

pub const SAFE_MINIMIZE_MESSAGE: u32 = WM_APP + 0x054D;

static ALLOW_NEXT_NATIVE_MINIMIZE: AtomicBool = AtomicBool::new(false);
static SAFE_MINIMIZE_POSTED: AtomicBool = AtomicBool::new(false);
static SCREENSHOT_REQUESTED: AtomicBool = AtomicBool::new(false);

static NEXT_SCREENSHOT_ID: AtomicU64 = AtomicU64::new(1);
static PENDING_SCREENSHOT_ID: AtomicU64 = AtomicU64::new(0);
static SCREENSHOT_IN_FLIGHT_ID: AtomicU64 = AtomicU64::new(0);
static MINIMIZE_AFTER_SCREENSHOT_ID: AtomicU64 = AtomicU64::new(0);
static LAST_HWND: AtomicIsize = AtomicIsize::new(0);
static READY_MINIMIZE_HWND: AtomicIsize = AtomicIsize::new(0);
static LAST_FRAME: Mutex<Option<PreviewFrame>> = Mutex::new(None);

#[derive(Debug)]
struct TaskbarMinimizeScreenshot {
    request_id: u64,
}

struct PreviewFrame {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

pub fn request_minimize_with_real_preview(hwnd: HWND) {
    if hwnd.is_invalid() || unsafe { IsIconic(hwnd).as_bool() } {
        return;
    }

    LAST_HWND.store(hwnd.0 as isize, Ordering::Release);
    if capture_visible_window_frame(hwnd) {
        post_safe_minimize_after_present(hwnd);
        return;
    }

    let in_flight_id = SCREENSHOT_IN_FLIGHT_ID.load(Ordering::Acquire);
    if in_flight_id != 0 {
        MINIMIZE_AFTER_SCREENSHOT_ID.store(in_flight_id, Ordering::Release);
        return;
    }

    if let Some(request_id) = request_screenshot(hwnd, true) {
        MINIMIZE_AFTER_SCREENSHOT_ID.store(request_id, Ordering::Release);
    }
}

pub fn take_screenshot_request() -> bool {
    SCREENSHOT_REQUESTED.swap(false, Ordering::AcqRel)
}

pub fn is_minimize_after_screenshot_pending() -> bool {
    MINIMIZE_AFTER_SCREENSHOT_ID.load(Ordering::Acquire) != 0
}

pub fn screenshot_user_data() -> egui::UserData {
    egui::UserData::new(TaskbarMinimizeScreenshot {
        request_id: PENDING_SCREENSHOT_ID.load(Ordering::Acquire),
    })
}

pub fn handle_screenshot_event(user_data: &egui::UserData, image: &egui::ColorImage) -> bool {
    let Some(request_id) = taskbar_minimize_screenshot_id(user_data) else {
        return false;
    };

    let in_flight_id = SCREENSHOT_IN_FLIGHT_ID.load(Ordering::Acquire);
    if in_flight_id != 0 && request_id < in_flight_id {
        return true;
    }
    if request_id == in_flight_id {
        let _ = SCREENSHOT_IN_FLIGHT_ID.compare_exchange(
            in_flight_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    let minimize_pending = MINIMIZE_AFTER_SCREENSHOT_ID.load(Ordering::Acquire) == request_id;
    if minimize_pending && !store_screenshot(image) {
        return false;
    }

    if minimize_pending {
        let raw = LAST_HWND.swap(0, Ordering::AcqRel);
        if raw != 0 {
            READY_MINIMIZE_HWND.store(raw, Ordering::Release);
        }
        let _ = MINIMIZE_AFTER_SCREENSHOT_ID.compare_exchange(
            request_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    true
}

pub fn take_minimize_hwnd_after_screenshot() -> Option<HWND> {
    let raw = READY_MINIMIZE_HWND.swap(0, Ordering::AcqRel);
    (raw != 0).then_some(HWND(raw as *mut core::ffi::c_void))
}

pub fn post_safe_minimize_after_present(hwnd: HWND) {
    if hwnd.is_invalid() || unsafe { IsIconic(hwnd).as_bool() } {
        return;
    }

    if SAFE_MINIMIZE_POSTED.swap(true, Ordering::AcqRel) {
        return;
    }

    let posted =
        unsafe { PostMessageW(Some(hwnd), SAFE_MINIMIZE_MESSAGE, WPARAM(0), LPARAM(0)) }.is_ok();
    if !posted {
        SAFE_MINIMIZE_POSTED.store(false, Ordering::Release);
    }
}

pub fn consume_allowed_native_minimize() -> bool {
    ALLOW_NEXT_NATIVE_MINIMIZE.swap(false, Ordering::AcqRel)
}

pub fn perform_safe_minimize(hwnd: HWND, before_minimize: fn()) {
    SAFE_MINIMIZE_POSTED.store(false, Ordering::Release);

    if hwnd.is_invalid() || unsafe { IsIconic(hwnd).as_bool() } || !has_screenshot() {
        return;
    }

    // Provide iconic bitmaps to DWM so it can show a thumbnail for the
    // taskbar preview while the wgpu surface is unavailable during minimize.
    proactive_set_iconic_bitmaps(hwnd);
    set_has_iconic_bitmap(hwnd, true);
    before_minimize();

    ALLOW_NEXT_NATIVE_MINIMIZE.store(true, Ordering::Release);
    unsafe {
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
    }
    ALLOW_NEXT_NATIVE_MINIMIZE.store(false, Ordering::Release);
}

fn set_has_iconic_bitmap(hwnd: HWND, enabled: bool) {
    if hwnd.is_invalid() {
        return;
    }

    let value: i32 = if enabled { 1 } else { 0 };
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_HAS_ICONIC_BITMAP,
            &value as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<i32>() as u32,
        );
    }
}

pub fn handle_dwm_iconic_message(hwnd: HWND, msg: u32, lparam: LPARAM) -> bool {
    match msg {
        WM_DWMSENDICONICTHUMBNAIL => {
            let raw = lparam.0 as usize;
            let width = ((raw & 0xffff) as i32).max(1);
            let height = (((raw >> 16) & 0xffff) as i32).max(1);
            set_iconic_thumbnail(hwnd, width, height)
        }
        WM_DWMSENDICONICLIVEPREVIEWBITMAP => set_iconic_live_preview(hwnd),
        _ => false,
    }
}

fn taskbar_minimize_screenshot_id(user_data: &egui::UserData) -> Option<u64> {
    user_data
        .data
        .as_deref()
        .and_then(|data| data.downcast_ref::<TaskbarMinimizeScreenshot>())
        .map(|data| data.request_id)
}

fn request_screenshot(hwnd: HWND, force: bool) -> Option<u64> {
    if hwnd.is_invalid() || unsafe { IsIconic(hwnd).as_bool() } {
        return None;
    }

    if !force && SCREENSHOT_IN_FLIGHT_ID.load(Ordering::Acquire) != 0 {
        return None;
    }
    let request_id = NEXT_SCREENSHOT_ID.fetch_add(1, Ordering::AcqRel);
    SCREENSHOT_IN_FLIGHT_ID.store(request_id, Ordering::Release);
    PENDING_SCREENSHOT_ID.store(request_id, Ordering::Release);
    SCREENSHOT_REQUESTED.store(true, Ordering::Release);

    unsafe {
        if PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)).is_err() {
            let _ = SCREENSHOT_IN_FLIGHT_ID.compare_exchange(
                request_id,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return None;
        }
    }

    Some(request_id)
}

fn store_screenshot(image: &egui::ColorImage) -> bool {
    let [width, height] = image.size;
    if width == 0 || height == 0 || image.pixels.len() != width.saturating_mul(height) {
        return false;
    }

    let pixels = image
        .pixels
        .iter()
        .map(|color| {
            let [r, g, b, _a] = color.to_srgba_unmultiplied();
            rgb(r, g, b)
        })
        .collect();

    if let Ok(mut frame) = LAST_FRAME.lock() {
        *frame = Some(PreviewFrame {
            width,
            height,
            pixels,
        });
        return true;
    }

    false
}

fn capture_visible_window_frame(hwnd: HWND) -> bool {
    let Some((width, height, pixels)) = capture_visible_window_pixels(hwnd) else {
        return false;
    };

    if let Ok(mut frame) = LAST_FRAME.lock() {
        *frame = Some(PreviewFrame {
            width,
            height,
            pixels,
        });
        return true;
    }

    false
}

fn capture_visible_window_pixels(hwnd: HWND) -> Option<(usize, usize, Vec<u32>)> {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut rect).ok()?;
    }

    let source_width = rect.right.checked_sub(rect.left)?;
    let source_height = rect.bottom.checked_sub(rect.top)?;
    if source_width <= 100 || source_height <= 100 {
        return None;
    }

    let (capture_width, capture_height) = fit_inside(source_width, source_height, 1600, 1000);
    let width = capture_width as usize;
    let height = capture_height as usize;
    if width > 8192 || height > 8192 {
        return None;
    }

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }

        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        if mem_dc.is_invalid() {
            ReleaseDC(None, screen_dc);
            return None;
        }

        let mut bits = std::ptr::null_mut();
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: capture_width,
                biHeight: -capture_height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: (width as u32)
                    .saturating_mul(height as u32)
                    .saturating_mul(4),
                ..Default::default()
            },
            ..Default::default()
        };

        let bitmap =
            match CreateDIBSection(Some(screen_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(_) => {
                    let _ = DeleteDC(mem_dc);
                    ReleaseDC(None, screen_dc);
                    return None;
                }
            };

        if bitmap.is_invalid() || bits.is_null() {
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
            return None;
        }

        let previous_bitmap = SelectObject(mem_dc, HGDIOBJ::from(bitmap));
        let rop = SRCCOPY;
        let blit_ok = if capture_width == source_width && capture_height == source_height {
            BitBlt(
                mem_dc,
                0,
                0,
                capture_width,
                capture_height,
                Some(screen_dc),
                rect.left,
                rect.top,
                rop,
            )
            .is_ok()
        } else {
            StretchBlt(
                mem_dc,
                0,
                0,
                capture_width,
                capture_height,
                Some(screen_dc),
                rect.left,
                rect.top,
                source_width,
                source_height,
                rop,
            )
            .as_bool()
        };

        let pixels = if blit_ok {
            let src = std::slice::from_raw_parts(bits.cast::<u32>(), width * height);
            Some(src.iter().map(|pixel| pixel | 0xff00_0000).collect())
        } else {
            None
        };

        let _ = SelectObject(mem_dc, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        pixels.map(|pixels| (width, height, pixels))
    }
}

fn has_screenshot() -> bool {
    LAST_FRAME
        .lock()
        .is_ok_and(|frame| frame.as_ref().is_some_and(|f| !f.pixels.is_empty()))
}

fn proactive_set_iconic_bitmaps(hwnd: HWND) {
    let Some(frame) = LAST_FRAME.lock().ok().and_then(|frame| frame.clone()) else {
        return;
    };

    let (tw, th) = fit_inside(frame.width as i32, frame.height as i32, 400, 300);
    if let Some(bitmap) = create_preview_bitmap_from_frame(&frame, tw, th) {
        unsafe {
            let _ = DwmSetIconicThumbnail(hwnd, bitmap, 0);
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
        }
    }

    let (lw, lh) = fit_inside(frame.width as i32, frame.height as i32, 1600, 1000);
    if let Some(bitmap) = create_preview_bitmap_from_frame(&frame, lw, lh) {
        unsafe {
            let _ = DwmSetIconicLivePreviewBitmap(hwnd, bitmap, None, 0);
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
        }
    }
}

fn set_iconic_thumbnail(hwnd: HWND, max_width: i32, max_height: i32) -> bool {
    let Some(frame) = LAST_FRAME.lock().ok().and_then(|frame| frame.clone()) else {
        return false;
    };
    let (width, height) = fit_inside(
        frame.width as i32,
        frame.height as i32,
        max_width,
        max_height,
    );
    let Some(bitmap) = create_preview_bitmap_from_frame(&frame, width, height) else {
        return false;
    };

    unsafe {
        let _ = DwmSetIconicThumbnail(hwnd, bitmap, 0);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
    }
    true
}

fn set_iconic_live_preview(hwnd: HWND) -> bool {
    let Some(frame) = LAST_FRAME.lock().ok().and_then(|frame| frame.clone()) else {
        return false;
    };
    let (width, height) = fit_inside(frame.width as i32, frame.height as i32, 1600, 1000);
    let Some(bitmap) = create_preview_bitmap_from_frame(&frame, width, height) else {
        return false;
    };

    unsafe {
        let _ = DwmSetIconicLivePreviewBitmap(hwnd, bitmap, None, 0);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
    }
    true
}

fn fit_inside(width: i32, height: i32, max_width: i32, max_height: i32) -> (i32, i32) {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = (max_width.max(1) as f32 / width)
        .min(max_height.max(1) as f32 / height)
        .min(1.0);
    (
        (width * scale).round().max(1.0) as i32,
        (height * scale).round().max(1.0) as i32,
    )
}

fn create_preview_bitmap_from_frame(
    frame: &PreviewFrame,
    width: i32,
    height: i32,
) -> Option<windows::Win32::Graphics::Gdi::HBITMAP> {
    let width = width.clamp(1, 1600);
    let height = height.clamp(1, 1000);
    let mut bits = std::ptr::null_mut();
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (width as u32)
                .saturating_mul(height as u32)
                .saturating_mul(4),
            ..Default::default()
        },
        ..Default::default()
    };

    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()? };
    if bits.is_null() || bitmap.is_invalid() {
        return None;
    }

    let dst =
        unsafe { std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize) };
    scale_frame_nearest(frame, dst, width as usize, height as usize);
    Some(bitmap)
}

fn scale_frame_nearest(frame: &PreviewFrame, dst: &mut [u32], dst_w: usize, dst_h: usize) {
    if frame.width == 0 || frame.height == 0 || dst_w == 0 || dst_h == 0 {
        return;
    }

    for y in 0..dst_h {
        let src_y = (y * frame.height / dst_h).min(frame.height - 1);
        for x in 0..dst_w {
            let src_x = (x * frame.width / dst_w).min(frame.width - 1);
            dst[y * dst_w + x] = frame.pixels[src_y * frame.width + src_x];
        }
    }
}

#[inline]
fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

impl Clone for PreviewFrame {
    fn clone(&self) -> Self {
        Self {
            width: self.width,
            height: self.height,
            pixels: self.pixels.clone(),
        }
    }
}
