use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, PostMessageW,
    RegisterWindowMessageW, SendMessageW, SetForegroundWindow, ShowWindow, TrackPopupMenu, HICON,
    ICON_SMALL, IDI_APPLICATION, MF_STRING, SW_HIDE, SW_RESTORE, SW_SHOW, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_CLOSE, WM_GETICON, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP,
};

pub(crate) const TRAY_CALLBACK_MESSAGE: u32 = 0x8000 + 0x36;

const TRAY_ICON_ID: u32 = 1;
const MENU_OPEN_ID: usize = 1001;
const MENU_EXIT_ID: usize = 1002;
const ACTION_NONE: u8 = 0;
const ACTION_OPEN: u8 = 1;
const ACTION_EXIT: u8 = 2;

static TRAY_ICON_ADDED: AtomicBool = AtomicBool::new(false);
static PENDING_ACTION: AtomicU8 = AtomicU8::new(ACTION_NONE);
static TASKBAR_CREATED_MESSAGE: OnceLock<u32> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Open,
    Exit,
}

pub(crate) fn add_tray_icon(hwnd: HWND) -> bool {
    if TRAY_ICON_ADDED.load(Ordering::Acquire) {
        return true;
    }

    let mut data = notify_icon_data(hwnd);
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &mut data).as_bool() };
    if added {
        TRAY_ICON_ADDED.store(true, Ordering::Release);
    }
    added
}

pub(crate) fn remove_tray_icon(hwnd: HWND) {
    if !TRAY_ICON_ADDED.swap(false, Ordering::AcqRel) {
        return;
    }
    let mut data = notify_icon_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &mut data);
    }
}

pub(crate) fn take_action() -> Option<TrayAction> {
    match PENDING_ACTION.swap(ACTION_NONE, Ordering::AcqRel) {
        ACTION_OPEN => Some(TrayAction::Open),
        ACTION_EXIT => Some(TrayAction::Exit),
        _ => None,
    }
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
        WM_LBUTTONDBLCLK => {
            show_window(hwnd);
            PENDING_ACTION.store(ACTION_OPEN, Ordering::Release);
        }
        WM_RBUTTONUP => show_context_menu(hwnd),
        _ => {}
    }
}

pub(crate) fn hide_window(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub(crate) fn show_window(hwnd: HWND) {
    remove_tray_icon(hwnd);
    unsafe {
        if windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

fn notify_icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW::default();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ICON_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_CALLBACK_MESSAGE;
    data.hIcon = window_icon(hwnd);
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

fn show_context_menu(hwnd: HWND) {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return;
    };
    let open_label = wide(&rust_i18n::t!("tray.open").to_string());
    let exit_label = wide(&rust_i18n::t!("tray.exit").to_string());
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_ID, PCWSTR(open_label.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT_ID, PCWSTR(exit_label.as_ptr()));
        let _ = SetForegroundWindow(hwnd);
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_ok() {
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                hwnd,
                None,
            )
            .0 as usize;
            match command {
                MENU_OPEN_ID => {
                    show_window(hwnd);
                    PENDING_ACTION.store(ACTION_OPEN, Ordering::Release);
                }
                MENU_EXIT_ID => {
                    PENDING_ACTION.store(ACTION_EXIT, Ordering::Release);
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                _ => {}
            }
        }
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
