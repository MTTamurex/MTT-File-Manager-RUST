use super::*;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::LRESULT;
use windows::Win32::UI::WindowsAndMessaging::*;

extern "system" fn receive(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    received: usize,
) -> LRESULT {
    if message == CHANGE_MESSAGE {
        handle_change_message(wparam, lparam);
        if take_change_notification() {
            // The flag lives on the registering thread until its window is destroyed.
            unsafe { *(received as *mut bool) = true };
        }
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

#[test]
fn recycle_notification_locks_real_shell_delivery() {
    // Notify a temporary folder, never the user's Recycle Bin or actual files.
    let _com = super::super::ComApartmentGuard::init_sta_best_effort();
    let directory = tempfile::tempdir().unwrap();
    let path: Vec<u16> = directory
        .path()
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut received = false;
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(SetWindowSubclass(
            hwnd,
            Some(receive),
            1,
            (&mut received as *mut bool) as usize,
        )
        .as_bool());
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(
            windows::core::PCWSTR(path.as_ptr()),
            None,
            &mut pidl,
            0,
            None,
        )
        .unwrap();
        let entry = SHChangeNotifyEntry {
            pidl,
            fRecursive: true.into(),
        };
        let id = SHChangeNotifyRegister(
            hwnd,
            SHCNRF_ShellLevel | SHCNRF_NewDelivery,
            SHCNE_UPDATEDIR.0 as i32,
            CHANGE_MESSAGE,
            1,
            &entry,
        );
        SHChangeNotify(
            SHCNE_UPDATEDIR,
            SHCNF_IDLIST | SHCNF_FLUSHNOWAIT,
            Some(pidl.cast()),
            None,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !received && Instant::now() < deadline {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, Some(hwnd), 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
            std::thread::yield_now();
        }
        if id != 0 {
            let _ = SHChangeNotifyDeregister(id);
        }
        CoTaskMemFree(Some(pidl.cast()));
        DestroyWindow(hwnd).unwrap();
        assert_ne!(id, 0);
        assert!(received, "Shell notification was not locked and consumed");
    }
}

use std::os::windows::ffi::OsStrExt;
