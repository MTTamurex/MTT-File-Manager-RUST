//! A thread-bound, process-shared lease. Windows releases it if the owner dies.
use super::super::owned_handle::OwnedHandle;
use std::marker::PhantomData;
use std::rc::Rc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

pub(super) struct Ownership {
    handle: OwnedHandle,
    acquired: bool,
    _thread: PhantomData<Rc<()>>,
}

impl Ownership {
    pub(super) fn new(name: PCWSTR) -> windows::core::Result<Self> {
        let handle = unsafe { CreateMutexW(None, false, name)? };
        Ok(Self {
            handle: OwnedHandle::new(handle).expect("CreateMutexW returned a valid handle"),
            acquired: false,
            _thread: PhantomData,
        })
    }

    pub(super) fn acquire(&mut self) -> bool {
        if !self.acquired {
            let result = unsafe { WaitForSingleObject(self.handle.as_raw(), 0) };
            self.acquired = result == WAIT_OBJECT_0 || result == WAIT_ABANDONED;
        }
        self.acquired
    }
}

impl Drop for Ownership {
    fn drop(&mut self) {
        if self.acquired {
            unsafe {
                let _ = ReleaseMutex(self.handle.as_raw());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn tray_ownership_is_exclusive_and_transfers_after_release() {
        let name = format!("Local\\MTT.Tray.Test.Release.{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut first = Ownership::new(PCWSTR(wide.as_ptr())).unwrap();
        assert!(first.acquire());
        assert!(first.acquire()); // must not recursively acquire the mutex
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let other = std::thread::spawn(move || {
            let mut next = Ownership::new(PCWSTR(wide.as_ptr())).unwrap();
            assert!(!next.acquire());
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            assert!(next.acquire());
        });
        ready_rx.recv().unwrap();
        drop(first);
        release_tx.send(()).unwrap();
        other.join().unwrap();
    }

    #[test]
    fn tray_ownership_recovers_an_abandoned_mutex() {
        let name = format!("Local\\MTT.Tray.Test.Abandoned.{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // Keep an open successor handle so the named kernel object survives.
        let mut successor = Ownership::new(PCWSTR(wide.as_ptr())).unwrap();
        let raw = std::thread::spawn(move || {
            let handle = unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())).unwrap() };
            assert_eq!(unsafe { WaitForSingleObject(handle, 0) }, WAIT_OBJECT_0);
            // Deliberately leave without releasing, simulating an owner crash.
            handle.0 as usize
        })
        .join()
        .unwrap();
        assert!(successor.acquire());
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(windows::Win32::Foundation::HANDLE(
                raw as *mut _,
            ));
        }
    }
}
