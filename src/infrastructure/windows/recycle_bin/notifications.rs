use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::WM_APP;

pub(crate) const CHANGE_MESSAGE: u32 = WM_APP + 0x3A2;

static REGISTRATION_ID: AtomicU32 = AtomicU32::new(0);
static CHANGE_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn register_change_notifications(hwnd: HWND) -> windows::core::Result<()> {
    if REGISTRATION_ID.load(Ordering::Acquire) != 0 {
        return Ok(());
    }

    unsafe {
        let recycle_bin_pidl = SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, 0, None)?;
        let entry = SHChangeNotifyEntry {
            pidl: recycle_bin_pidl,
            fRecursive: windows::core::BOOL(1),
        };
        let registration_id = SHChangeNotifyRegister(
            hwnd,
            SHCNRF_ShellLevel | SHCNRF_InterruptLevel | SHCNRF_NewDelivery,
            SHCNE_ALLEVENTS.0 as i32,
            CHANGE_MESSAGE,
            1,
            &entry,
        );
        CoTaskMemFree(Some(recycle_bin_pidl as *mut _));

        if registration_id == 0 {
            return Err(windows::core::Error::from_thread());
        }

        if REGISTRATION_ID
            .compare_exchange(0, registration_id, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            let _ = SHChangeNotifyDeregister(registration_id);
        }
    }

    Ok(())
}

pub(crate) fn handle_change_message(wparam: WPARAM, lparam: LPARAM) {
    unsafe {
        // NewDelivery supplies the shared-memory handle in wParam and its
        // owning process ID in lParam (not necessarily this process).
        let notification_handle = HANDLE(wparam.0 as *mut _);
        let mut pidl_array = std::ptr::null_mut();
        let mut event = 0;
        let lock = SHChangeNotification_Lock(
            notification_handle,
            lparam.0 as u32,
            Some(&mut pidl_array),
            Some(&mut event),
        );
        if lock.0.is_null() {
            return;
        }
        let _ = SHChangeNotification_Unlock(lock);
    }
    CHANGE_PENDING.store(true, Ordering::Release);
}

pub(crate) fn take_change_notification() -> bool {
    CHANGE_PENDING.swap(false, Ordering::AcqRel)
}

pub(crate) fn unregister_change_notifications() {
    let registration_id = REGISTRATION_ID.swap(0, Ordering::AcqRel);
    if registration_id != 0 {
        unsafe {
            let _ = SHChangeNotifyDeregister(registration_id);
        }
    }
}

#[cfg(test)]
#[path = "notification_tests.rs"]
mod tests;
