use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::OnceLock;

mod badge;
mod clicks;
mod commands;
pub(crate) mod coordinator;
mod graphics;
mod labels;
mod menu;
mod menu_icons;
mod menu_text;
pub(crate) mod model;
mod native_popup;
mod notification;
mod ownership;
mod popup;
mod protocol;
pub(crate) mod session_exit;
#[cfg(test)]
mod tests;
mod window_ref;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    RegisterWindowMessageW, SetForegroundWindow, ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOW,
};

pub(crate) const TRAY_CALLBACK_MESSAGE: u32 = 0x8000 + 0x36;
pub(crate) const POPUP_CONTINUE_MESSAGE: u32 = 0x8000 + 0x39;

pub(crate) fn continue_popup(hwnd: HWND) {
    popup::handoff::show_pending(hwnd);
}

pub(crate) use notification::{
    remove_tray_icon, restore_icon_after_taskbar_restart, update_tray_icon,
};

pub(crate) fn warn_exit_failure(hwnd: HWND) {
    commands::warn_failure(hwnd, true);
}
pub(crate) fn warn_restore_failure(hwnd: HWND) {
    commands::warn_restore_failure(hwnd);
}
thread_local! {
    static PENDING_ACTIONS: RefCell<VecDeque<TrayAction>> = const { RefCell::new(VecDeque::new()) };
}
static TASKBAR_CREATED_MESSAGE: OnceLock<u32> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Open,
    Exit,
    Activate(model::Target),
    ExitAll,
    CaptureSession {
        requester: usize,
        process_id: u32,
        token: String,
    },
    ExitFailed,
}

pub(crate) fn take_action() -> Option<TrayAction> {
    PENDING_ACTIONS.with(|q| q.borrow_mut().pop_front())
}

fn queue_action(action: TrayAction) -> bool {
    PENDING_ACTIONS.with(|q| {
        let mut queue = q.borrow_mut();
        if action == TrayAction::Exit {
            queue.clear();
            queue.push_back(action);
        } else if queue.len() < 64 && !queue.contains(&TrayAction::Exit) {
            queue.push_back(action);
        } else {
            return false;
        }
        true
    })
}

pub(crate) unsafe fn receive_packet(sender: HWND, data: LPARAM) -> Option<bool> {
    unsafe { protocol::receive(sender, data) }
}

pub(crate) fn taskbar_created_message() -> u32 {
    *TASKBAR_CREATED_MESSAGE.get_or_init(|| {
        let message_name: Vec<u16> = "TaskbarCreated"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe { RegisterWindowMessageW(PCWSTR(message_name.as_ptr())) }
    })
}

pub(crate) fn handle_callback(hwnd: HWND, event: LPARAM) {
    clicks::handle(hwnd, event.0 as u32);
}

pub(crate) fn hide_window(hwnd: HWND) {
    coordinator::mark_hidden();
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub(crate) fn show_window(hwnd: HWND) {
    coordinator::mark_visible();
    unsafe {
        if windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

pub(crate) fn request_open(hwnd: HWND) {
    show_window(hwnd);
    queue_action(TrayAction::Open);
}
