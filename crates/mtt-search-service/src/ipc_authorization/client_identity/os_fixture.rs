//! Test-thread tokens and private pipes only. Never changes any existing DACL,
//! launches a process, or connects to the installed service.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LocalFree, ERROR_NO_TOKEN, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::ConvertStringSidToSidW;
use windows::Win32::Security::{
    CreateRestrictedToken, ImpersonateLoggedOnUser, RevertToSelf, DISABLE_MAX_PRIVILEGE, PSID,
    SID_AND_ATTRIBUTES, TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};
use windows::Win32::System::IO::CancelIoEx;

pub(super) fn assert_no_thread_token() {
    let mut handle = HANDLE::default();
    match unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut handle) } {
        Err(error) => assert_eq!(error.code(), ERROR_NO_TOKEN.to_hresult()),
        Ok(()) => {
            let _handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
            panic!("unexpected test-thread impersonation token");
        }
    }
}

pub(crate) fn with_restricting_sid<T>(action: impl FnOnce() -> T) -> T {
    assert_no_thread_token();
    let mut process_token = HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_DUPLICATE | TOKEN_QUERY,
            &mut process_token,
        )
    }
    .unwrap();
    let process_token = unsafe { OwnedHandle::from_raw_handle(process_token.0) };
    let mut sid = PSID::default();
    unsafe { ConvertStringSidToSidW(w!("S-1-5-12"), &mut sid) }.unwrap();
    struct SidAllocation(PSID);
    impl Drop for SidAllocation {
        fn drop(&mut self) {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(self.0 .0)));
            }
        }
    }
    let _sid = SidAllocation(sid);
    let restricting = [SID_AND_ATTRIBUTES {
        Sid: sid,
        Attributes: 0,
    }];
    let mut restricted = HANDLE::default();
    unsafe {
        CreateRestrictedToken(
            HANDLE(process_token.as_raw_handle()),
            DISABLE_MAX_PRIVILEGE,
            None,
            None,
            Some(&restricting),
            &mut restricted,
        )
    }
    .unwrap();
    let restricted = unsafe { OwnedHandle::from_raw_handle(restricted.0) };
    struct RevertGuard;
    impl Drop for RevertGuard {
        fn drop(&mut self) {
            if unsafe { RevertToSelf() }.is_err() {
                // Never return to the test runner with an unexpected token.
                std::process::abort();
            }
        }
    }
    unsafe { ImpersonateLoggedOnUser(HANDLE(restricted.as_raw_handle())) }.unwrap();
    let guard = RevertGuard;
    let result = action();
    drop(guard);
    assert_no_thread_token();
    result
}

static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);

pub(super) struct IdentityPipe {
    server: OwnedHandle,
    client: Option<File>,
    done: mpsc::Sender<()>,
    watchdog: Option<JoinHandle<bool>>,
}

impl IdentityPipe {
    pub fn new(sqos_level: u32) -> Self {
        let name = format!(
            r"\\.\pipe\MTT.IdentityTest.{}.{}",
            std::process::id(),
            NEXT_PIPE.fetch_add(1, Ordering::Relaxed)
        );
        assert_ne!(name, mtt_search_protocol::PIPE_NAME);
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let raw = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!raw.is_invalid(), "create private identity-test pipe");
        let server = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        let cancellation = server.try_clone().unwrap();
        let (done, receiver) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(Duration::from_secs(5)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                let pipe = HANDLE(cancellation.as_raw_handle());
                unsafe {
                    let _ = CancelIoEx(pipe, None);
                    let _ = DisconnectNamedPipe(pipe);
                }
                true
            } else {
                false
            }
        });
        // Establish owning cleanup before connection/read operations can panic.
        let mut pipe = Self {
            server,
            client: None,
            done,
            watchdog: Some(watchdog),
        };
        pipe.client = Some(
            OpenOptions::new()
                .read(true)
                .write(true)
                .access_mode(mtt_search_protocol::PIPE_CLIENT_ACCESS)
                .custom_flags(0x00100000 | sqos_level)
                .open(&name)
                .expect("open private identity-test client"),
        );
        match unsafe { ConnectNamedPipe(pipe.raw(), None) } {
            Ok(()) => {}
            Err(error) => assert_eq!(error.code(), ERROR_PIPE_CONNECTED.to_hresult()),
        }
        pipe.client.as_mut().unwrap().write_all(&[0x42]).unwrap();
        let mut byte = [0];
        let mut count = 0;
        unsafe { ReadFile(pipe.raw(), Some(&mut byte), Some(&mut count), None) }.unwrap();
        assert_eq!((byte, count), ([0x42], 1));
        assert_eq!(pipe.client_pid(), std::process::id());
        pipe
    }

    pub fn raw(&self) -> HANDLE {
        HANDLE(self.server.as_raw_handle())
    }

    pub fn client_pid(&self) -> u32 {
        let mut pid = 0;
        unsafe { GetNamedPipeClientProcessId(self.raw(), &mut pid) }.unwrap();
        pid
    }
}

impl Drop for IdentityPipe {
    fn drop(&mut self) {
        let _ = self.done.send(());
        if let Some(worker) = self.watchdog.take() {
            let timed_out = worker.join().expect("identity-test watchdog panicked");
            if !std::thread::panicking() {
                assert!(
                    !timed_out,
                    "private identity test exceeded its safety deadline"
                );
            }
        }
        unsafe {
            let _ = DisconnectNamedPipe(self.raw());
        }
    }
}
