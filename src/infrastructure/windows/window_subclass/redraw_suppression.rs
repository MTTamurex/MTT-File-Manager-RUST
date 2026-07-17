use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_INTERNALPAINT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, IsIconic, IsZoomed, PostMessageW, SendMessageW, WM_APP, WM_SIZE,
};

const SIZE_MINIMIZED: usize = 1;
const SIZE_RESTORED: usize = 0;
const SIZE_MAXIMIZED: usize = 2;
const SC_COMMAND_MASK: usize = 0xFFF0;
const SC_SIZE_CMD: usize = 0xF000;
const SC_MOVE_CMD: usize = 0xF010;
pub(super) const REPLAY_MESSAGE: u32 = WM_APP + 0x3A1;

static NEXT_SIZE_MOVE_IS_CAPTION: AtomicBool = AtomicBool::new(false);
static PAINT_SUPPRESSED: AtomicBool = AtomicBool::new(false);
static DEFERRED_SIZE_PENDING: AtomicBool = AtomicBool::new(false);

pub(super) fn note_non_client_press(is_caption: bool) {
    NEXT_SIZE_MOVE_IS_CAPTION.store(is_caption, Ordering::Release);
}

pub(super) fn note_system_command(wparam: usize) {
    if let Some(is_caption_move) = caption_move_from_system_command(wparam) {
        NEXT_SIZE_MOVE_IS_CAPTION.store(is_caption_move, Ordering::Release);
    }
}

pub(super) fn begin_caption_move() {
    if NEXT_SIZE_MOVE_IS_CAPTION.swap(false, Ordering::AcqRel) {
        PAINT_SUPPRESSED.store(true, Ordering::Release);
    }
}

pub(super) fn end_caption_move(hwnd: HWND) {
    NEXT_SIZE_MOVE_IS_CAPTION.store(false, Ordering::Release);
    restore_redraw(hwnd, true);
}

pub(super) fn clear_pending_move() {
    NEXT_SIZE_MOVE_IS_CAPTION.store(false, Ordering::Release);
}

pub(super) fn is_active() -> bool {
    PAINT_SUPPRESSED.load(Ordering::Acquire)
}

pub(super) fn defer_size() {
    DEFERRED_SIZE_PENDING.store(true, Ordering::Release);
}

pub(super) fn handle_replay(hwnd: HWND) {
    send_current_size(hwnd);
}

pub(super) fn reset_for_remove(hwnd: HWND) {
    NEXT_SIZE_MOVE_IS_CAPTION.store(false, Ordering::Release);
    restore_redraw(hwnd, false);
}

pub(super) fn reset_for_destroy() {
    NEXT_SIZE_MOVE_IS_CAPTION.store(false, Ordering::Release);
    PAINT_SUPPRESSED.store(false, Ordering::Release);
    DEFERRED_SIZE_PENDING.store(false, Ordering::Release);
}

fn restore_redraw(hwnd: HWND, invalidate: bool) {
    if !PAINT_SUPPRESSED.swap(false, Ordering::AcqRel) {
        return;
    }

    if invalidate {
        if !replay_deferred_size(hwnd) {
            unsafe {
                let _ = RedrawWindow(Some(hwnd), None, None, RDW_INTERNALPAINT);
            }
        }
    } else {
        DEFERRED_SIZE_PENDING.store(false, Ordering::Release);
    }
}

fn replay_deferred_size(hwnd: HWND) -> bool {
    if !DEFERRED_SIZE_PENDING.swap(false, Ordering::AcqRel) {
        return false;
    }

    if let Err(error) = unsafe { PostMessageW(Some(hwnd), REPLAY_MESSAGE, WPARAM(0), LPARAM(0)) } {
        log::warn!("Failed to queue deferred window size replay: {error}");
        send_current_size(hwnd);
    }
    true
}

fn send_current_size(hwnd: HWND) {
    let mut client_rect = windows::Win32::Foundation::RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client_rect).is_err() } {
        return;
    }

    let width = (client_rect.right - client_rect.left).clamp(0, u16::MAX as i32) as usize;
    let height = (client_rect.bottom - client_rect.top).clamp(0, u16::MAX as i32) as usize;
    let size_type = if unsafe { IsIconic(hwnd).as_bool() } {
        SIZE_MINIMIZED
    } else if unsafe { IsZoomed(hwnd).as_bool() } {
        SIZE_MAXIMIZED
    } else {
        SIZE_RESTORED
    };
    let packed_size = LPARAM(((height << 16) | width) as isize);

    unsafe {
        let _ = SendMessageW(hwnd, WM_SIZE, Some(WPARAM(size_type)), Some(packed_size));
    }
}

fn caption_move_from_system_command(wparam: usize) -> Option<bool> {
    match wparam & SC_COMMAND_MASK {
        SC_MOVE_CMD => Some(true),
        SC_SIZE_CMD => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_commands_distinguish_caption_move_from_resize() {
        assert_eq!(caption_move_from_system_command(SC_MOVE_CMD), Some(true));
        assert_eq!(caption_move_from_system_command(SC_SIZE_CMD), Some(false));
    }

    #[test]
    fn system_command_classification_ignores_low_bits() {
        assert_eq!(
            caption_move_from_system_command(SC_MOVE_CMD | 0x0002),
            Some(true)
        );
        assert_eq!(caption_move_from_system_command(0xF020), None);
    }
}
