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

mod instance_number;
pub(crate) use instance_number::get as instance_number;

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

/// Complete startup or release its lock on failure so a failed window cannot
/// block every subsequent launch. The caller must close on failure.
pub(crate) fn initialize_main_window(hwnd: HWND) -> windows::core::Result<()> {
    finish_startup(
        || super::window_subclass::install_borderless_subclass(hwnd),
        || register_main_window(hwnd),
    )
}

fn finish_startup(
    install_receiver: impl FnOnce() -> bool,
    register: impl FnOnce() -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    let _startup_lock = STARTUP_LOCK.with(|lock| lock.borrow_mut().take());
    if !install_receiver() {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(0x80004005u32 as i32),
            "Could not install the main window message receiver",
        ));
    }
    register()
}

/// Call only after the main window's message receiver has been installed.
fn register_main_window(hwnd: HWND) -> windows::core::Result<()> {
    instance_number::assign(hwnd, &main_windows())?;
    unsafe {
        SetPropW(
            hwnd,
            w!("MTT.FileManager.MainWindow"),
            Some(HANDLE(std::ptr::dangling_mut::<u8>().cast())),
        )
    }
}

pub fn find_main_window() -> Option<HWND> {
    main_windows().into_iter().next()
}

pub(crate) fn main_windows() -> Vec<HWND> {
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        if !unsafe { GetPropW(hwnd, w!("MTT.FileManager.MainWindow")) }.is_invalid() {
            unsafe {
                (*(data.0 as *mut Vec<HWND>)).push(hwnd);
            }
        }
        true.into()
    }
    let mut found = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(&mut found as *mut Vec<HWND> as isize));
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
