use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_HAS_ICONIC_BITMAP, DwmSetIconicLivePreviewBitmap, DwmSetIconicThumbnail,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
    StretchBlt,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, PostMessageW, SW_MINIMIZE, ShowWindow, WM_APP,
    WM_DWMSENDICONICLIVEPREVIEWBITMAP, WM_DWMSENDICONICTHUMBNAIL, WM_NCDESTROY, WM_SYSCOMMAND,
};
const SAFE_MINIMIZE_MESSAGE: u32 = WM_APP + 0x054D;
const PREVIEW_SUBCLASS_ID: usize = 1;
const SC_COMMAND_MASK: usize = 0xFFF0;
const SC_MINIMIZE_CMD: usize = 0xF020;

static ALLOW_NATIVE_MINIMIZE: AtomicBool = AtomicBool::new(false);
static LAST_FRAME: Mutex<Option<PreviewFrame>> = Mutex::new(None);

pub(super) fn install(hwnd: HWND) -> Result<(), String> {
    let installed =
        unsafe { SetWindowSubclass(hwnd, Some(preview_subclass_proc), PREVIEW_SUBCLASS_ID, 0) };
    installed
        .as_bool()
        .then_some(())
        .ok_or_else(|| "SetWindowSubclass returned FALSE".to_owned())
}

#[derive(Clone)]
struct PreviewFrame {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

pub(super) fn request(hwnd: HWND) {
    if hwnd.is_invalid() || unsafe { IsIconic(hwnd).as_bool() } {
        return;
    }

    if !capture_visible_window_frame(hwnd) {
        eprintln!("GDI capture failed; using ShowWindow without a preview");
        unsafe {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
        return;
    }

    if unsafe { PostMessageW(Some(hwnd), SAFE_MINIMIZE_MESSAGE, WPARAM(0), LPARAM(0)) }.is_err() {
        perform_preview_minimize(hwnd);
    }
}

fn perform_preview_minimize(hwnd: HWND) {
    proactive_set_iconic_bitmaps(hwnd);
    set_has_iconic_bitmap(hwnd, true);

    ALLOW_NATIVE_MINIMIZE.store(true, Ordering::Release);
    unsafe {
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
    }
    ALLOW_NATIVE_MINIMIZE.store(false, Ordering::Release);
}

extern "system" fn preview_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(preview_subclass_proc), PREVIEW_SUBCLASS_ID);
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
    }

    if msg == SAFE_MINIMIZE_MESSAGE {
        perform_preview_minimize(hwnd);
        return LRESULT(0);
    }

    if handle_dwm_iconic_message(hwnd, msg, lparam) {
        return LRESULT(0);
    }

    if msg == WM_SYSCOMMAND
        && wparam.0 & SC_COMMAND_MASK == SC_MINIMIZE_CMD
        && !ALLOW_NATIVE_MINIMIZE.swap(false, Ordering::AcqRel)
    {
        request(hwnd);
        return LRESULT(0);
    }

    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn set_has_iconic_bitmap(hwnd: HWND, enabled: bool) {
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

fn handle_dwm_iconic_message(hwnd: HWND, msg: u32, lparam: LPARAM) -> bool {
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

fn capture_visible_window_frame(hwnd: HWND) -> bool {
    let Some((width, height, pixels)) = capture_visible_window_pixels(hwnd) else {
        return false;
    };
    let Ok(mut frame) = LAST_FRAME.lock() else {
        return false;
    };
    *frame = Some(PreviewFrame {
        width,
        height,
        pixels,
    });
    true
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
        let copied = if capture_width == source_width && capture_height == source_height {
            BitBlt(
                mem_dc,
                0,
                0,
                capture_width,
                capture_height,
                Some(screen_dc),
                rect.left,
                rect.top,
                SRCCOPY,
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
                SRCCOPY,
            )
            .as_bool()
        };
        let pixels = copied.then(|| {
            std::slice::from_raw_parts(bits.cast::<u32>(), width * height)
                .iter()
                .map(|pixel| pixel | 0xff00_0000)
                .collect()
        });

        let _ = SelectObject(mem_dc, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);
        pixels.map(|pixels| (width, height, pixels))
    }
}

fn proactive_set_iconic_bitmaps(hwnd: HWND) {
    let Some(frame) = cloned_last_frame() else {
        return;
    };

    let (thumbnail_width, thumbnail_height) =
        fit_inside(frame.width as i32, frame.height as i32, 400, 300);
    if let Some(bitmap) = create_preview_bitmap(&frame, thumbnail_width, thumbnail_height) {
        unsafe {
            let _ = DwmSetIconicThumbnail(hwnd, bitmap, 0);
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
        }
    }

    let (preview_width, preview_height) =
        fit_inside(frame.width as i32, frame.height as i32, 1600, 1000);
    if let Some(bitmap) = create_preview_bitmap(&frame, preview_width, preview_height) {
        unsafe {
            let _ = DwmSetIconicLivePreviewBitmap(hwnd, bitmap, None, 0);
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
        }
    }
}

fn set_iconic_thumbnail(hwnd: HWND, max_width: i32, max_height: i32) -> bool {
    let Some(frame) = cloned_last_frame() else {
        return false;
    };
    let (width, height) = fit_inside(
        frame.width as i32,
        frame.height as i32,
        max_width,
        max_height,
    );
    let Some(bitmap) = create_preview_bitmap(&frame, width, height) else {
        return false;
    };
    unsafe {
        let _ = DwmSetIconicThumbnail(hwnd, bitmap, 0);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
    }
    true
}

fn set_iconic_live_preview(hwnd: HWND) -> bool {
    let Some(frame) = cloned_last_frame() else {
        return false;
    };
    let (width, height) = fit_inside(frame.width as i32, frame.height as i32, 1600, 1000);
    let Some(bitmap) = create_preview_bitmap(&frame, width, height) else {
        return false;
    };
    unsafe {
        let _ = DwmSetIconicLivePreviewBitmap(hwnd, bitmap, None, 0);
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
    }
    true
}

fn cloned_last_frame() -> Option<PreviewFrame> {
    LAST_FRAME.lock().ok()?.as_ref().cloned()
}

fn create_preview_bitmap(
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
    if bitmap.is_invalid() || bits.is_null() {
        if !bitmap.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ::from(bitmap));
            }
        }
        return None;
    }

    let destination =
        unsafe { std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * height) as usize) };
    for y in 0..height as usize {
        let source_y = (y * frame.height / height as usize).min(frame.height - 1);
        for x in 0..width as usize {
            let source_x = (x * frame.width / width as usize).min(frame.width - 1);
            destination[y * width as usize + x] = frame.pixels[source_y * frame.width + source_x];
        }
    }
    Some(bitmap)
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
