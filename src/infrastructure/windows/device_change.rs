use eframe::egui;
use std::{
    ffi::c_void,
    sync::{mpsc::Sender, OnceLock},
};
use windows::{
    core::{Result, PCWSTR},
    Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME,
    Win32::System::LibraryLoader::GetModuleHandleW,
    Win32::System::Threading::GetCurrentThreadId,
    Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostThreadMessageW,
        RegisterClassW, RegisterDeviceNotificationW, TranslateMessage, DBT_DEVICEARRIVAL,
        DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE,
        DEV_BROADCAST_DEVICEINTERFACE_W, HDEVNOTIFY, HWND_MESSAGE, MSG, WINDOW_EX_STYLE,
        WINDOW_STYLE, WM_DEVICECHANGE, WM_QUIT, WNDCLASSW,
    },
};

const CLASS_NAME_WIDE: [u16; 22] = [
    77, 84, 84, 68, 101, 118, 105, 99, 101, 67, 104, 97, 110, 103, 101, 87, 105, 110, 100, 111,
    119, 0,
];

static DEVICE_EVENT_SENDER: OnceLock<Sender<()>> = OnceLock::new();
static EGUI_CONTEXT: OnceLock<egui::Context> = OnceLock::new();
static DEVICE_LISTENER_THREAD_ID: OnceLock<u32> = OnceLock::new();

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
            Some(HWND_MESSAGE),
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
    }

    Ok(())
}

pub fn shutdown_device_change_listener() {
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

    let _notification_handle: HDEVNOTIFY = RegisterDeviceNotificationW(
        HANDLE(hwnd.0),
        &mut filter as *mut _ as *mut c_void,
        DEVICE_NOTIFY_WINDOW_HANDLE,
    )?;
    Ok(())
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

            if event == DBT_DEVICEARRIVAL || event == DBT_DEVICEREMOVECOMPLETE {
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
