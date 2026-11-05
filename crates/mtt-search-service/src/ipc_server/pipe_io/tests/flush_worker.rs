//! Cancellable flush worker for isolated tests, never a production connection.

use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::FlushFileBuffers;
use windows::Win32::System::Pipes::DisconnectNamedPipe;
use windows::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};

const COMPLETION_DEADLINE: Duration = Duration::from_secs(2);

pub(super) struct FlushWorker {
    server: OwnedHandle,
    finished: Receiver<Result<(), i32>>,
    worker: Option<JoinHandle<()>>,
}

impl FlushWorker {
    pub fn start(server: OwnedHandle) -> Self {
        let flush_server = server.try_clone().expect("duplicate flush handle");
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let result = unsafe { FlushFileBuffers(HANDLE(flush_server.as_raw_handle())) }
                .map_err(|error| error.code().0);
            let _ = finished_tx.send(result);
        });
        let task = Self {
            server,
            finished,
            worker: Some(worker),
        };
        // Construct the owning guard before waiting, so panic/timeout also
        // cancels and joins the worker instead of leaking a blocked thread.
        started_rx
            .recv_timeout(COMPLETION_DEADLINE)
            .expect("isolated flush worker did not start");
        task
    }

    pub fn assert_pending(&self, observation: Duration) {
        assert!(
            matches!(
                self.finished.recv_timeout(observation),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "buffered flush should wait while the client has unread reply bytes"
        );
    }

    pub fn wait(&self) -> Result<(), i32> {
        self.finished
            .recv_timeout(COMPLETION_DEADLINE)
            .expect("isolated flush did not complete within the safety deadline")
    }

    pub fn cancel(&self) {
        if let Some(worker) = &self.worker {
            unsafe {
                let _ = CancelSynchronousIo(HANDLE(worker.as_raw_handle()));
                let server = HANDLE(self.server.as_raw_handle());
                let _ = CancelIoEx(server, None);
                // Also handles cancellation racing the start of FlushFileBuffers:
                // a later flush on this disconnected instance cannot wait on a reader.
                let _ = DisconnectNamedPipe(server);
            }
        }
    }
}

impl Drop for FlushWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(worker) = self.worker.take() {
            if let Err(payload) = worker.join() {
                if !std::thread::panicking() {
                    std::panic::resume_unwind(payload);
                }
            }
        }
    }
}
