//! Windows shell operations.

mod context_menu;
mod file_op;
mod shfile_ops;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

pub use context_menu::{
    open_with_dialog, open_with_shell, show_shell_context_menu, ContextMenuResult,
};
pub use file_op::{
    copy_item_with_file_op, copy_items_with_file_op, move_item_with_file_op,
    move_items_with_file_op,
};
pub use shfile_ops::{
    copy_item_with_shell, copy_items_with_shell, delete_item_with_shell,
    delete_items_permanently_with_shell, delete_items_with_shell, move_item_with_shell,
    move_items_with_shell, rename_item_with_shell,
};

/// Creates a hidden, unowned top-level popup window used as the owner for Shell
/// file-operation progress dialogs (copy, move, delete).
///
/// By giving Shell dialogs this invisible proxy as their owner instead of the
/// real app window, we prevent the Shell from disabling the app window while a
/// long operation (e.g. a large move/copy) is running or being cancelled.
/// The proxy has no owner (`hwndParent = None`), so disabling it does not
/// cascade to the app window.
///
/// The window is 0x0, never shown, and excluded from the taskbar/Alt+Tab
/// via `WS_EX_TOOLWINDOW`. It lives for the entire process lifetime.
///
/// **Must be called on the same thread that will pass the returned HWND to
/// Shell file-operation APIs.** This ensures the proxy window and the Shell
/// progress dialog are on the same thread, avoiding cross-thread
/// `SendMessage` marshaling that can cause UI thread starvation.
pub fn create_shell_op_proxy_window() -> Option<HWND> {
    let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();

    unsafe {
        match CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        ) {
            Ok(h) if !h.is_invalid() => Some(h),
            _ => None,
        }
    }
}
