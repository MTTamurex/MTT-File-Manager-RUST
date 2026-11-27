//! Switches a surface between an owned popup and a child of its owner.
//!
//! A popup is composed by DWM on its own, so it cannot move in step with the
//! owner's maximize/restore animation or its lagging content during a live
//! resize. While such a change runs, the surface is hosted as a child, which
//! DWM composes together with the owner.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetParent, SetWindowLongPtrW, GWLP_HWNDPARENT, GWL_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_POPUP,
};

fn set_style(hwnd: HWND, popup: bool) {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let style = if popup {
        (style & !(WS_CHILD.0 | WS_CLIPSIBLINGS.0)) | WS_POPUP.0
    } else {
        (style & !WS_POPUP.0) | WS_CHILD.0 | WS_CLIPSIBLINGS.0
    };
    unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize) };
}

/// Re-hosts `hwnd` as a child of `owner`. The caller positions it in client
/// coordinates afterwards.
pub(super) fn to_child(hwnd: HWND, owner: HWND) -> bool {
    // WS_CHILD must be set before SetParent attaches the window.
    set_style(hwnd, false);
    if unsafe { SetParent(hwnd, Some(owner)) }.is_ok() {
        return true;
    }
    set_style(hwnd, true);
    false
}

/// Re-hosts `hwnd` as a popup owned by `owner`. The caller positions it in
/// screen coordinates afterwards.
pub(super) fn to_popup(hwnd: HWND, owner: HWND) -> bool {
    if unsafe { SetParent(hwnd, None) }.is_err() {
        return false;
    }
    // Detached windows must drop WS_CHILD after SetParent; the owner keeps it
    // above the main window and hides it on minimize.
    set_style(hwnd, true);
    unsafe { SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as isize) };
    true
}
