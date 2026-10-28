use super::{badge, graphics::Icon};
use std::{cell::RefCell, mem::size_of};
use windows::core::GUID;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    LoadIconW, SendMessageW, HICON, ICON_SMALL, IDI_APPLICATION, WM_GETICON,
};

pub(super) const TRAY_GUID: GUID = GUID::from_u128(0x664bb25c_9eef_47ef_9459_15ad93815cd8);
// Shell scales this supersampled icon to the notification area's physical size.
const ICON_SIZE: u32 = 64;

#[derive(Clone, PartialEq, Eq)]
struct Presentation {
    count: usize,
    tooltip: String,
}

#[derive(Default)]
struct State {
    added: bool,
    presentation: Option<Presentation>,
    icon: Option<Icon>,
    epoch: u64,
}
thread_local! { static STATE: RefCell<State> = RefCell::new(State::default()); }

pub(crate) fn update_tray_icon(hwnd: HWND, count: usize) -> bool {
    update_with(hwnd, count, |operation, data| unsafe {
        Shell_NotifyIconW(operation, data).as_bool()
    })
}

fn update_with(
    hwnd: HWND,
    count: usize,
    mut notify: impl FnMut(windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE, &NOTIFYICONDATAW) -> bool,
) -> bool {
    let desired = Presentation {
        count,
        tooltip: format!(
            "MTT File Manager — {}",
            rust_i18n::t!("tray.windows_count", count = count)
        ),
    };
    let (added, epoch, unchanged) = STATE.with(|s| {
        let state = s.borrow();
        (
            state.added,
            state.epoch,
            state.presentation.as_ref() == Some(&desired),
        )
    });
    if added && unchanged {
        return true;
    }
    let icon = badge::render(count, ICON_SIZE).and_then(|pixels| Icon::from_pixmap(&pixels));
    let mut data = identity(hwnd);
    data.hIcon = icon
        .as_ref()
        .map_or_else(|| window_icon(hwnd), |icon| icon.0);
    set_tooltip(&mut data, &desired.tooltip);
    // No RefCell borrow is held across shell calls, which can dispatch other
    // sent window messages while waiting for Explorer.
    let success = if added {
        notify(NIM_MODIFY, &data)
    } else if notify(NIM_ADD, &data) {
        true
    } else {
        // The lease holder alone may replace a stale crashed owner's GUID.
        notify(NIM_DELETE, &data);
        notify(NIM_ADD, &data)
    };
    if success {
        STATE.with(|s| {
            let mut state = s.borrow_mut();
            if state.epoch == epoch {
                state.added = true;
                // Retry artwork creation after temporary GDI allocation failure.
                state.presentation = icon.as_ref().map(|_| desired);
                state.icon = icon;
            }
        });
    }
    // A failed counter refresh must not make a previously usable tray icon
    // unavailable. Keep the old artwork and retry on the next coordination tick.
    STATE.with(|s| s.borrow().added)
}

pub(crate) fn remove_tray_icon(hwnd: HWND) {
    let (added, icon) = STATE.with(|s| {
        let mut state = s.borrow_mut();
        let added = state.added;
        state.added = false;
        state.presentation = None;
        state.epoch = state.epoch.wrapping_add(1);
        (added, state.icon.take())
    });
    if added {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &identity(hwnd));
        }
    }
    drop(icon);
}

pub(crate) fn restore_icon_after_taskbar_restart(hwnd: HWND) {
    let was_added = invalidate_registration();
    if was_added
        && super::coordinator::owns_icon(hwnd)
        && !update_tray_icon(hwnd, super::coordinator::window_count())
    {
        log::warn!("[TRAY] Could not restore the notification-area icon after Explorer restarted");
    }
}

fn invalidate_registration() -> bool {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let was_added = state.added;
        state.added = false;
        state.presentation = None;
        state.epoch = state.epoch.wrapping_add(1);
        was_added
    })
}

fn identity(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        guidItem: TRAY_GUID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_GUID,
        uCallbackMessage: super::TRAY_CALLBACK_MESSAGE,
        ..Default::default()
    }
}

fn set_tooltip(data: &mut NOTIFYICONDATAW, text: &str) {
    // Do not split a surrogate pair or omit the terminating NUL.
    let mut length = 0;
    for character in text.chars() {
        let mut units = [0u16; 2];
        let units = character.encode_utf16(&mut units);
        if length + units.len() >= data.szTip.len() {
            break;
        }
        data.szTip[length..length + units.len()].copy_from_slice(units);
        length += units.len();
    }
    data.szTip[length] = 0;
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
        HICON(response.0 as *mut _)
    } else {
        unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() }
    }
}

#[cfg(test)]
mod tests;
