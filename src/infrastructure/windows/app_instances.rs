//! Main-window discovery and serialized startup. Viewer/helper processes do not
//! participate. Every main window is registered, including multi-instance ones.
use super::owned_handle::OwnedHandle;
use std::cell::RefCell;
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetPropW, SendMessageTimeoutW, SetPropW, SMTO_ABORTIFHUNG, SMTO_BLOCK,
    SMTO_ERRORONEXIT,
};

pub const ALLOW_MULTIPLE_INSTANCES_KEY: &str = "allow_multiple_instances";
pub(crate) const RESTORE_MESSAGE: u32 = 0x8000 + 0x37;

thread_local! {
    static STARTUP_LOCK: RefCell<Option<StartupLock>> = const { RefCell::new(None) };
}

pub fn allows_multiple_instances(value: Option<&str>) -> bool {
    value == Some("true")
}

/// Owned only by the startup/UI thread. The OS releases an abandoned mutex if
/// startup crashes, so no persistent lock file or stale PID needs cleanup.
pub struct StartupLock(OwnedHandle, std::marker::PhantomData<std::rc::Rc<()>>);

impl StartupLock {
    pub fn acquire() -> windows::core::Result<Self> {
        Self::acquire_named(w!("Local\\MTT.FileManager.Startup"))
    }

    fn acquire_named(name: PCWSTR) -> windows::core::Result<Self> {
        let handle = unsafe { CreateMutexW(None, false, name)? };
        let handle = OwnedHandle::new(handle).expect("CreateMutexW returned a valid handle");
        match unsafe { WaitForSingleObject(handle.as_raw(), 30_000) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle, std::marker::PhantomData)),
            _ => Err(windows::core::Error::new(
                windows::core::HRESULT(0x800705B4u32 as i32),
                "Timed out waiting for the file manager to finish starting",
            )),
        }
    }

    pub fn hold_until_window_ready(self) {
        STARTUP_LOCK.with(|lock| *lock.borrow_mut() = Some(self));
    }
}

impl Drop for StartupLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0.as_raw());
        }
    }
}

/// Call only after the main window's message receiver has been installed.
pub(crate) fn register_main_window(hwnd: HWND) {
    let result = unsafe {
        SetPropW(
            hwnd,
            w!("MTT.FileManager.MainWindow"),
            Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
        )
    };
    if let Err(error) = result {
        log::error!("[INSTANCES] Could not register the main window: {error}");
        return;
    }
    STARTUP_LOCK.with(|lock| drop(lock.borrow_mut().take()));
}

pub fn find_main_window() -> Option<HWND> {
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        if !unsafe { GetPropW(hwnd, w!("MTT.FileManager.MainWindow")) }.is_invalid() {
            unsafe {
                *(data.0 as *mut Option<HWND>) = Some(hwnd);
            }
            return false.into();
        }
        true.into()
    }
    let mut found = None;
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut found as *mut Option<HWND> as isize));
    }
    found
}

/// A bounded synchronous message also wakes a hidden/tray window. Foreground
/// restoration is repeated by the newly launched process while it has focus.
pub fn restore_main_window(hwnd: HWND) -> bool {
    let mut acknowledged = 0;
    let result = unsafe {
        SendMessageTimeoutW(
            hwnd,
            RESTORE_MESSAGE,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
            2_000,
            Some(&mut acknowledged),
        )
    };
    if result.0 != 0 && acknowledged == 1 {
        super::drives::restore_window_foreground(hwnd);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests;
