use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, IsZoomed, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION,
    HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
};

const RESIZE_BORDER_WIDTH: i32 = 8;

static NATIVE_CAPTION_DRAG_ENABLED: AtomicBool = AtomicBool::new(true);
static CAPTION_DRAG_REGION_VALID: AtomicBool = AtomicBool::new(false);
static CAPTION_DRAG_REGION_X: AtomicI32 = AtomicI32::new(0);
static CAPTION_DRAG_REGION_Y: AtomicI32 = AtomicI32::new(0);
static CAPTION_DRAG_REGION_W: AtomicI32 = AtomicI32::new(0);
static CAPTION_DRAG_REGION_H: AtomicI32 = AtomicI32::new(0);

#[inline]
pub fn is_native_caption_drag_enabled() -> bool {
    NATIVE_CAPTION_DRAG_ENABLED.load(Ordering::Relaxed)
}

pub fn set_native_caption_drag_enabled(enabled: bool) {
    NATIVE_CAPTION_DRAG_ENABLED.store(enabled, Ordering::SeqCst);
    if !enabled {
        clear_caption_drag_region();
    }
}

pub fn set_caption_drag_region_px(x: i32, y: i32, width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        clear_caption_drag_region();
        return;
    }

    CAPTION_DRAG_REGION_X.store(x, Ordering::Relaxed);
    CAPTION_DRAG_REGION_Y.store(y, Ordering::Relaxed);
    CAPTION_DRAG_REGION_W.store(width, Ordering::Relaxed);
    CAPTION_DRAG_REGION_H.store(height, Ordering::Relaxed);
    CAPTION_DRAG_REGION_VALID.store(true, Ordering::Release);
}

pub fn clear_caption_drag_region() {
    CAPTION_DRAG_REGION_VALID.store(false, Ordering::Release);
}

pub(super) fn is_caption_code(code: usize) -> bool {
    code == HTCAPTION as usize
}

pub(super) fn handle(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let is_zoomed = unsafe { IsZoomed(hwnd).as_bool() };
    let cursor_x = (lparam.0 as i32) & 0xFFFF;
    let cursor_y = ((lparam.0 as i32) >> 16) & 0xFFFF;
    let cursor_x = if cursor_x > 32767 {
        cursor_x - 65536
    } else {
        cursor_x
    };
    let cursor_y = if cursor_y > 32767 {
        cursor_y - 65536
    } else {
        cursor_y
    };

    let mut client_rect = windows::Win32::Foundation::RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client_rect).is_err() } {
        return LRESULT(HTCLIENT as isize);
    }

    let mut window_rect = windows::Win32::Foundation::RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window_rect).is_err() } {
        return LRESULT(HTCLIENT as isize);
    }

    let x = cursor_x - window_rect.left;
    let y = cursor_y - window_rect.top;
    let width = window_rect.right - window_rect.left;
    let height = window_rect.bottom - window_rect.top;

    if x < 0 || y < 0 || x >= width || y >= height {
        return LRESULT(HTCLIENT as isize);
    }

    if !is_zoomed {
        let on_left = x < RESIZE_BORDER_WIDTH;
        let on_right = x >= width - RESIZE_BORDER_WIDTH;
        let on_top = y < RESIZE_BORDER_WIDTH;
        let on_bottom = y >= height - RESIZE_BORDER_WIDTH;

        let resize_hit = if on_top && on_left {
            Some(HTTOPLEFT)
        } else if on_top && on_right {
            Some(HTTOPRIGHT)
        } else if on_bottom && on_left {
            Some(HTBOTTOMLEFT)
        } else if on_bottom && on_right {
            Some(HTBOTTOMRIGHT)
        } else if on_left {
            Some(HTLEFT)
        } else if on_right {
            Some(HTRIGHT)
        } else if on_top {
            Some(HTTOP)
        } else if on_bottom {
            Some(HTBOTTOM)
        } else {
            None
        };

        if let Some(hit) = resize_hit {
            return LRESULT(hit as isize);
        }
    }

    if is_native_caption_drag_enabled() && point_in_caption_drag_region(x, y) {
        return LRESULT(HTCAPTION as isize);
    }

    LRESULT(HTCLIENT as isize)
}

#[inline]
fn point_in_caption_drag_region(x: i32, y: i32) -> bool {
    if !CAPTION_DRAG_REGION_VALID.load(Ordering::Acquire) {
        return false;
    }

    let rx = CAPTION_DRAG_REGION_X.load(Ordering::Relaxed);
    let ry = CAPTION_DRAG_REGION_Y.load(Ordering::Relaxed);
    let rw = CAPTION_DRAG_REGION_W.load(Ordering::Relaxed);
    let rh = CAPTION_DRAG_REGION_H.load(Ordering::Relaxed);

    if rw <= 0 || rh <= 0 {
        return false;
    }

    let right = rx.saturating_add(rw);
    let bottom = ry.saturating_add(rh);
    x >= rx && y >= ry && x < right && y < bottom
}

#[cfg(test)]
mod tests {
    use super::RESIZE_BORDER_WIDTH;

    const _: () = {
        assert!(RESIZE_BORDER_WIDTH > 0);
        assert!(RESIZE_BORDER_WIDTH <= 20);
    };
}
