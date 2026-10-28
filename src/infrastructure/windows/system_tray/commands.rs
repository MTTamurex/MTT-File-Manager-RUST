//! User commands require acknowledgement; periodic snapshots can simply retry.
use super::{
    coordinator,
    model::{Packet, Target},
    protocol,
    window_ref::WindowRef,
    TrayAction,
};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetWindowThreadProcessId, MessageBoxW, MB_ICONWARNING, MB_OK,
};

pub(super) fn activate(from: HWND, to: WindowRef, target: Target) -> bool {
    if !to.is_current() {
        return false;
    }
    if from == to.hwnd {
        if !super::queue_action(TrayAction::Activate(target)) {
            return false;
        }
        super::show_window(to.hwnd);
        true
    } else {
        // Transfer the foreground permission granted by the user's tray click.
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(to.hwnd, Some(&mut pid));
            let _ = AllowSetForegroundWindow(pid);
        }
        protocol::send(from, to.hwnd, &Packet::Activate(target))
    }
}

pub(super) fn exit_window(from: HWND, to: WindowRef) -> bool {
    if !to.is_current() {
        return false;
    }
    if from == to.hwnd {
        coordinator::request_exit(from)
    } else {
        protocol::request_exit(from, to.hwnd)
    }
}

pub(super) fn exit_all(_from: HWND) -> bool {
    super::session_exit::request_all()
}

pub(super) fn exit_windows(
    owner: WindowRef,
    windows: &[WindowRef],
    mut send: impl FnMut(WindowRef) -> bool,
) -> bool {
    // Keep the tray owner alive until every peer has accepted its exit request.
    // Failed requests never set properties on other processes. This preserves
    // access to unresponsive/blocked peers and prevents a premature handoff.
    for &window in windows {
        if window != owner && window.is_current() && !send(window) && window.is_current() {
            return false;
        }
    }
    send(owner)
}

pub(super) fn warn_failure(hwnd: HWND, global_exit: bool) {
    log::warn!("[TRAY] A user command was not acknowledged");
    let message = if global_exit && crate::app::instance_sessions::restoring() {
        rust_i18n::t!("tray.restore_busy")
    } else if global_exit {
        rust_i18n::t!("tray.exit_failed")
    } else {
        rust_i18n::t!("tray.command_failed")
    };
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "MTT File Manager".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}

pub(super) fn warn_restore_failure(hwnd: HWND) {
    let message: Vec<u16> = rust_i18n::t!("tray.restore_failed")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let title: Vec<u16> = "MTT File Manager".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}

#[cfg(test)]
mod tests;
