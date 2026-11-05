use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows::Win32::Storage::FileSystem::FlushFileBuffers;
use windows::Win32::System::Pipes::DisconnectNamedPipe;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};
use windows::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};

use super::{watchdog_timed_out, WatchdogEvent};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ConnectionOutcome {
    Finished,
    TimedOut,
    HandlerPanicked,
}

struct Connection(OwnedHandle);

impl Connection {
    fn raw(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.raw());
        }
        // OwnedHandle closes only after the deadline guard has joined its worker.
    }
}

struct DeadlineGuard {
    events: Sender<WatchdogEvent>,
    worker: Option<JoinHandle<bool>>,
}

impl DeadlineGuard {
    fn finish(&mut self) -> bool {
        let _ = self.events.send(WatchdogEvent::Done);
        self.worker
            .take()
            .map(|worker| worker.join().unwrap_or(true))
            .unwrap_or(false)
    }
}

impl Drop for DeadlineGuard {
    fn drop(&mut self) {
        self.finish();
    }
}

fn current_thread_handle() -> Result<OwnedHandle, String> {
    let mut handle = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            GetCurrentThread(),
            GetCurrentProcess(),
            &mut handle,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(|error| format!("Cannot duplicate IPC thread handle: {error}"))?;
        Ok(OwnedHandle::from_raw_handle(handle.0))
    }
}

/// Own one connected instance through handler work, reply delivery and cleanup.
/// The existing operation-specific extensions also cover response flushing.
pub(super) fn serve(
    pipe: OwnedHandle,
    timeout: Duration,
    handler: impl FnOnce(HANDLE, &Sender<WatchdogEvent>),
) -> Result<ConnectionOutcome, String> {
    let connection = Connection(pipe);
    let cancellation_pipe = connection
        .0
        .try_clone()
        .map_err(|error| format!("Cannot duplicate IPC cancellation handle: {error}"))?;
    let handler_thread = current_thread_handle()?;
    let timed_out = Arc::new(AtomicBool::new(false));
    let timeout_flag = Arc::clone(&timed_out);
    let (events, receiver) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("search-ipc-deadline".into())
        .spawn(move || {
            let expired = watchdog_timed_out(&receiver, timeout);
            if expired {
                timeout_flag.store(true, Ordering::Release);
                unsafe {
                    // Cancel both synchronous flush/write calls and outstanding
                    // overlapped I/O before disconnecting this exact instance.
                    let _ = CancelSynchronousIo(HANDLE(handler_thread.as_raw_handle()));
                    let pipe = HANDLE(cancellation_pipe.as_raw_handle());
                    let _ = CancelIoEx(pipe, None);
                    // Covers a race where cancellation precedes the next I/O call.
                    let _ = DisconnectNamedPipe(pipe);
                }
            }
            expired
        })
        .map_err(|error| format!("Cannot start IPC deadline worker: {error}"))?;
    let mut deadline = DeadlineGuard {
        events,
        worker: Some(worker),
    };

    let handler_panicked = catch_unwind(AssertUnwindSafe(|| {
        handler(connection.raw(), &deadline.events);
    }))
    .is_err();
    if handler_panicked {
        eprintln!("[IPC] Client handler panicked; closing connection");
    }

    if !timed_out.load(Ordering::Acquire) {
        unsafe {
            // Keep the deadline active until buffered replies are consumed.
            // Disconnecting immediately here could silently discard valid replies.
            let _ = FlushFileBuffers(connection.raw());
        }
    }
    let expired = deadline.finish();
    // DeadlineGuard is now joined, so it can never act on a recycled handle or
    // cancel another operation on this thread after we return.
    Ok(if expired {
        ConnectionOutcome::TimedOut
    } else if handler_panicked {
        ConnectionOutcome::HandlerPanicked
    } else {
        ConnectionOutcome::Finished
    })
}
