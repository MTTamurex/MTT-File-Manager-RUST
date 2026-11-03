use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::DisconnectNamedPipe;
use windows::Win32::System::IO::CancelIoEx;

use super::super::create_pipe_named;

static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);

pub(super) fn unique_name() -> String {
    format!(
        r"\\.\pipe\MTT.Search.IpcTest.{}.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_PIPE.fetch_add(1, Ordering::Relaxed),
    )
}

pub(super) struct Connection {
    server: OwnedHandle,
    pub client: File,
    done: mpsc::Sender<()>,
    watchdog: Option<JoinHandle<bool>>,
}

impl Connection {
    pub fn new() -> Self {
        let name = unique_name();
        assert_ne!(name, mtt_search_protocol::PIPE_NAME);
        let raw = create_pipe_named(&name, true).expect("create isolated pipe");
        let server = unsafe { OwnedHandle::from_raw_handle(raw.0) };
        // Identification only: the fixture does not grant impersonation rights.
        let client = OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(mtt_search_protocol::PIPE_CLIENT_ACCESS)
            .custom_flags(0x00100000 | 0x00010000)
            .open(&name)
            .expect("connect to isolated pipe");

        let watchdog_server = server.try_clone().expect("duplicate watchdog handle");
        let (done, receiver) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(Duration::from_secs(5)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                let handle = HANDLE(watchdog_server.as_raw_handle());
                unsafe {
                    let _ = CancelIoEx(handle, None);
                    let _ = DisconnectNamedPipe(handle);
                }
                true
            } else {
                false
            }
        });

        let connection = Self {
            server,
            client,
            done,
            watchdog: Some(watchdog),
        };
        // The client already opened this instance; exercise the production
        // ERROR_PIPE_CONNECTED path without waiting for another process.
        assert!(super::super::super::wait_for_client(
            connection.server(),
            &Arc::new(AtomicBool::new(false)),
        ));
        connection
    }

    pub fn server(&self) -> HANDLE {
        HANDLE(self.server.as_raw_handle())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.done.send(());
        if let Some(watchdog) = self.watchdog.take() {
            let timed_out = watchdog.join().expect("watchdog thread panicked");
            if !std::thread::panicking() {
                assert!(!timed_out, "isolated pipe test exceeded its I/O deadline");
            }
        }
        unsafe {
            let _ = DisconnectNamedPipe(self.server());
        }
    }
}
