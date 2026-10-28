use super::model::Packet;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, WM_COPYDATA,
};

const MAGIC: usize = 0x4D545438;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const SNAPSHOT_TIMEOUT_MS: u32 = 150;
const COMMAND_TIMEOUT_MS: u32 = 2_000;

pub(super) fn send(from: HWND, to: HWND, packet: &Packet) -> bool {
    let timeout = match packet {
        Packet::Snapshot(_) => SNAPSHOT_TIMEOUT_MS,
        _ => COMMAND_TIMEOUT_MS,
    };
    send_with_timeout(from, to, packet, timeout)
}

fn send_with_timeout(from: HWND, to: HWND, packet: &Packet, timeout: u32) -> bool {
    let Ok(bytes) = serde_json::to_vec(packet) else {
        return false;
    };
    if bytes.len() > MAX_BYTES {
        return false;
    }
    let data = COPYDATASTRUCT {
        dwData: MAGIC,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr().cast_mut().cast(),
    };
    let mut acknowledged = 0;
    // Do not use SMTO_BLOCK: a receiver may be sending its own snapshot.
    let result = unsafe {
        SendMessageTimeoutW(
            to,
            WM_COPYDATA,
            WPARAM(from.0 as usize),
            LPARAM(&data as *const _ as isize),
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            timeout,
            Some(&mut acknowledged),
        )
    };
    result.0 != 0 && acknowledged == 1
}

pub(super) fn request_exit(from: HWND, to: HWND) -> bool {
    let mut acknowledged = 0;
    let result = unsafe {
        SendMessageTimeoutW(
            to,
            super::coordinator::EXIT_MESSAGE,
            WPARAM(from.0 as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
            COMMAND_TIMEOUT_MS,
            Some(&mut acknowledged),
        )
    };
    result.0 != 0 && acknowledged == 1
}

/// The caller supplies the OS-managed WM_COPYDATA buffer, valid for this call.
pub(super) unsafe fn receive(sender: HWND, data: LPARAM) -> Option<bool> {
    let data = unsafe { (data.0 as *const COPYDATASTRUCT).as_ref() }?;
    if data.dwData != MAGIC {
        return None;
    }
    if super::window_ref::WindowRef::capture(sender).is_none()
        || data.cbData == 0
        || data.cbData as usize > MAX_BYTES
        || data.lpData.is_null()
    {
        return Some(false);
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize) };
    let Ok(packet) = serde_json::from_slice::<Packet>(bytes) else {
        return Some(false);
    };
    match packet {
        Packet::RequestSession { token } => {
            return Some(super::session_exit::request_capture(sender, token))
        }
        Packet::SessionReply { token, session } => {
            return Some(super::session_exit::receive_reply(sender, &token, session))
        }
        Packet::ExitSessionGroup { token } => {
            return Some(super::session_exit::close(sender, &token))
        }
        Packet::SessionExitFailed { token } => return Some(super::session_exit::finish(&token)),
        Packet::Snapshot(snapshot) => super::coordinator::receive_snapshot(sender, snapshot),
        Packet::Activate(target) => {
            if !super::queue_action(super::TrayAction::Activate(target)) {
                return Some(false);
            }
            // Restore now so hidden eframe windows resume logic immediately.
            if let Some(hwnd) = super::coordinator::local_window() {
                super::show_window(hwnd);
            }
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests;
