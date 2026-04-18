use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

#[derive(Debug)]
pub struct OwnedHandle(HANDLE);

impl OwnedHandle {
    pub fn new(handle: HANDLE) -> Option<Self> {
        if handle.is_invalid() || handle == INVALID_HANDLE_VALUE {
            None
        } else {
            Some(Self(handle))
        }
    }

    pub fn as_raw(&self) -> HANDLE {
        self.0
    }

    pub fn into_raw(self) -> HANDLE {
        let handle = self.0;
        std::mem::forget(self);
        handle
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}