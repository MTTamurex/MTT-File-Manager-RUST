use std::cell::RefCell;
use std::collections::VecDeque;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

mod commands;
pub(crate) mod coordinator;
mod labels;
mod menu;
mod menu_text;
pub(crate) mod model;
mod ownership;
mod protocol;
#[cfg(test)]
mod tests;
mod window_ref;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    LoadIconW, RegisterWindowMessageW, SendMessageW, SetForegroundWindow, ShowWindow, HICON,
    ICON_SMALL, IDI_APPLICATION, SW_HIDE, SW_RESTORE, SW_SHOW, WM_GETICON, WM_LBUTTONUP,
    WM_RBUTTONUP,
};

pub(crate) const TRAY_CALLBACK_MESSAGE: u32 = 0x8000 + 0x36;

const TRAY_ICON_ID: u32 = 1;
// A stable shell identity lets a successor replace a crashed owner's icon,
// rather than leaving an indistinguishable stale notification-area entry.
const TRAY_GUID: GUID = GUID::from_u128(0x664bb25c_9eef_47ef_9459_15ad93815cd8);

static TRAY_ICON_ADDED: AtomicBool = AtomicBool::new(false);
thread_local! {
    static PENDING_ACTIONS: RefCell<VecDeque<TrayAction>> = const { RefCell::new(VecDeque::new()) };
}
static TASKBAR_CREATED_MESSAGE: OnceLock<u32> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Open,
    Exit,
    Activate(model::Target),
}

pub(crate) fn add_tray_icon(hwnd: HWND) -> bool {
    if TRAY_ICON_ADDED.load(Ordering::Acquire) {
        return true;
    }

    let data = notify_icon_data(hwnd);
    let added = unsafe {
        if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            true
        } else {
            // Only the mutex holder reaches this path. Delete by GUID to
            // discard any stale callback HWND before registering the new one.
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            Shell_NotifyIconW(NIM_ADD, &data).as_bool()
        }
    };
    if added {
        TRAY_ICON_ADDED.store(true, Ordering::Release);
    }
    added
}

pub(crate) fn remove_tray_icon(hwnd: HWND) {
    if !TRAY_ICON_ADDED.swap(false, Ordering::AcqRel) {
        return;
    }
    let data = notify_icon_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
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

pub(crate) fn restore_icon_after_taskbar_restart(hwnd: HWND) {
    if TRAY_ICON_ADDED.swap(false, Ordering::AcqRel) && !add_tray_icon(hwnd) {
        log::warn!("[TRAY] Could not restore the notification-area icon after Explorer restarted");
    }
}

pub(crate) fn handle_callback(hwnd: HWND, event: LPARAM) {
    match event.0 as u32 {
        WM_LBUTTONUP | WM_RBUTTONUP => menu::show(hwnd),
        _ => {}
    }
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

fn notify_icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID,
        guidItem: TRAY_GUID,
        uCallbackMessage: TRAY_CALLBACK_MESSAGE,
        hIcon: window_icon(hwnd),
        ..Default::default()
    };
    let tooltip: Vec<u16> = "MTT File Manager"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let tooltip_len = tooltip.len().min(data.szTip.len());
    data.szTip[..tooltip_len].copy_from_slice(&tooltip[..tooltip_len]);
    data
}

fn window_icon(hwnd: HWND) -> HICON {
    let response = unsafe {
        SendMessageW(
            hwnd,
            WM_GETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(0)),
        )
    };
    if response.0 != 0 {
        return HICON(response.0 as *mut _);
    }
    unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() }
}
