use eframe::egui;
use std::{
    ffi::c_void,
    sync::{mpsc::Sender, Mutex, OnceLock},
};
use windows::{
    core::{Result, PCWSTR},
    Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME,
    Win32::System::LibraryLoader::GetModuleHandleW,
    Win32::System::Threading::GetCurrentThreadId,
    Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostThreadMessageW,
        RegisterClassW, RegisterDeviceNotificationW, TranslateMessage,
        UnregisterDeviceNotification, DBT_DEVICEARRIVAL, DBT_DEVICEQUERYREMOVE,
        DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE,
        DEV_BROADCAST_DEVICEINTERFACE_W, HDEVNOTIFY, MSG, WINDOW_EX_STYLE, WINDOW_STYLE,
        WM_DEVICECHANGE, WM_QUIT, WNDCLASSW,
    },
};

const CLASS_NAME_WIDE: [u16; 22] = [
    77, 84, 84, 68, 101, 118, 105, 99, 101, 67, 104, 97, 110, 103, 101, 87, 105, 110, 100, 111,
    119, 0,
];

static DEVICE_EVENT_SENDER: OnceLock<Sender<()>> = OnceLock::new();
static EGUI_CONTEXT: OnceLock<egui::Context> = OnceLock::new();
static DEVICE_LISTENER_THREAD_ID: OnceLock<u32> = OnceLock::new();
static DEVICE_NOTIFICATION_HANDLE: Mutex<Option<DeviceNotificationHandle>> = Mutex::new(None);

struct DeviceNotificationHandle(HDEVNOTIFY);

impl Drop for DeviceNotificationHandle {
    fn drop(&mut self) {
        unsafe {
            if let Err(err) = UnregisterDeviceNotification(self.0) {
                log::debug!(
                    "[device_change] UnregisterDeviceNotification failed: {:?}",
                    err
                );
            }
        }
    }
}

unsafe impl Send for DeviceNotificationHandle {}

/// Starts a background thread that listens for WM_DEVICECHANGE events and notifies the UI
/// whenever a volume (drive) is mounted or unmounted.
pub fn start_device_change_listener(sender: Sender<()>, ctx: egui::Context) {
    std::thread::spawn(move || {
        if let Err(err) = run_device_listener(sender, ctx) {
            log::error!("[device_change] listener failed: {:?}", err);
        }
    });
}

fn run_device_listener(sender: Sender<()>, ctx: egui::Context) -> Result<()> {
    unsafe {
        if DEVICE_EVENT_SENDER.set(sender).is_err() {
            return Ok(()); // listener already initialized
        }

        if EGUI_CONTEXT.set(ctx).is_err() {
            return Ok(()); // context already set
        }

        let _ = DEVICE_LISTENER_THREAD_ID.set(GetCurrentThreadId());

        let hmodule = GetModuleHandleW(None)?;
        let hinstance = HINSTANCE(hmodule.0);
        let class_name = PCWSTR(CLASS_NAME_WIDE.as_ptr());

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(device_wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };

        // RegisterClassW returns 0 on failure or if the class already exists; ignore the error
        RegisterClassW(&wnd_class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            class_name,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None, // A hidden top-level window also receives volume unit-mask broadcasts.
            None,
            Some(hinstance),
            None,
        )?;

        register_volume_notifications(hwnd)?;

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, Some(HWND::default()), 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        clear_notification_handle();
    }

    Ok(())
}

pub fn shutdown_device_change_listener() {
    // Unregister device notification handle before stopping the message loop.
    clear_notification_handle();

    if let Some(thread_id) = DEVICE_LISTENER_THREAD_ID.get().copied() {
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe fn register_volume_notifications(hwnd: HWND) -> Result<()> {
    let mut filter = DEV_BROADCAST_DEVICEINTERFACE_W {
        dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
        dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
        dbcc_classguid: GUID_DEVINTERFACE_VOLUME,
        ..Default::default()
    };

    let notification_handle: HDEVNOTIFY = RegisterDeviceNotificationW(
        HANDLE(hwnd.0),
        &mut filter as *mut _ as *mut c_void,
        DEVICE_NOTIFY_WINDOW_HANDLE,
    )?;

    let mut slot = DEVICE_NOTIFICATION_HANDLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = Some(DeviceNotificationHandle(notification_handle));
    Ok(())
}

fn clear_notification_handle() {
    let mut slot = DEVICE_NOTIFICATION_HANDLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.take();
}

#[cfg(feature = "notify-watcher")]
pub(crate) fn repaint_after_watcher_lifecycle_change() {
    if let Some(ctx) = EGUI_CONTEXT.get() {
        ctx.request_repaint();
    }
}

unsafe extern "system" fn device_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DEVICECHANGE => {
            let event = wparam.0 as u32;

            if event == DBT_DEVICEQUERYREMOVE {
                // Do not leave the app's window recipient at LRESULT(0), which
                // is not the documented grant response for a removal query.
                // The search service independently releases its per-volume
                // index handles before acknowledging its PnP notification.
                return LRESULT(1);
            }

            if event == DBT_DEVICEARRIVAL || event == DBT_DEVICEREMOVECOMPLETE {
                #[cfg(feature = "notify-watcher")]
                if event == DBT_DEVICEARRIVAL {
                    resume_arriving_volumes(lparam);
                }
                if let Some(sender) = DEVICE_EVENT_SENDER.get() {
                    let _ = sender.send(());
                    // Force immediate UI repaint from worker thread
                    if let Some(ctx) = EGUI_CONTEXT.get() {
                        ctx.request_repaint();
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(feature = "notify-watcher")]
unsafe fn resume_arriving_volumes(lparam: LPARAM) {
    let Some(unitmask) = volume_arrival_unitmask(lparam) else {
        return;
    };
    for bit in 0..26 {
        if unitmask & (1 << bit) != 0 {
            crate::infrastructure::removal_aware_watcher::volume_arrived((b'A' + bit) as char);
        }
    }
}

#[cfg(feature = "notify-watcher")]
unsafe fn volume_arrival_unitmask(lparam: LPARAM) -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{
        DBT_DEVTYP_VOLUME, DEV_BROADCAST_HDR, DEV_BROADCAST_VOLUME,
    };
    if lparam.0 == 0 {
        return None;
    }
    let header = &*(lparam.0 as *const DEV_BROADCAST_HDR);
    if header.dbch_devicetype != DBT_DEVTYP_VOLUME
        || (header.dbch_size as usize) < std::mem::size_of::<DEV_BROADCAST_VOLUME>()
    {
        return None;
    }
    let volume = &*(lparam.0 as *const DEV_BROADCAST_VOLUME);
    Some(volume.dbcv_unitmask)
}

#[cfg(test)]
mod tests {
    use super::device_wnd_proc;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{DBT_DEVICEQUERYREMOVE, WM_DEVICECHANGE};

    #[test]
    fn volume_query_remove_is_explicitly_granted_by_the_app_window() {
        let result = unsafe {
            device_wnd_proc(
                HWND::default(),
                WM_DEVICECHANGE,
                WPARAM(DBT_DEVICEQUERYREMOVE as usize),
                LPARAM(0),
            )
        };

        assert_eq!(result.0, 1);
    }

    #[cfg(feature = "notify-watcher")]
    #[test]
    fn arrival_parser_uses_only_volume_unitmask_payloads() {
        use windows::Win32::UI::WindowsAndMessaging::{
            DBT_DEVTYP_DEVICEINTERFACE, DBT_DEVTYP_VOLUME,
        };
        let mask = (1u32 << 22) | (1u32 << 23);
        let parse = |payload: &[u32; 5]| unsafe {
            super::volume_arrival_unitmask(LPARAM(payload.as_ptr() as isize))
        };
        assert_eq!(parse(&[20, DBT_DEVTYP_VOLUME.0, 0, mask, 0]), Some(mask));
        assert_eq!(parse(&[12, DBT_DEVTYP_VOLUME.0, 0, mask, 0]), None);
        assert_eq!(parse(&[20, DBT_DEVTYP_DEVICEINTERFACE.0, 0, mask, 0]), None);
        assert_eq!(unsafe { super::volume_arrival_unitmask(LPARAM(0)) }, None);
    }
}
