//! Test safety wrapper around the real production connection lifecycle.

use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::DisconnectNamedPipe;
use windows::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};

use super::super::super::connection_lifecycle::{serve, ConnectionOutcome};
use super::super::super::WatchdogEvent;

pub(super) struct LifecycleWorker {
    server: OwnedHandle,
    result: Receiver<Result<ConnectionOutcome, String>>,
    worker: Option<JoinHandle<()>>,
}

impl LifecycleWorker {
    pub fn start(
        server: OwnedHandle,
        timeout: Duration,
        action: impl FnOnce(HANDLE, &Sender<WatchdogEvent>) + Send + 'static,
    ) -> Self {
        let worker_pipe = server.try_clone().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (result_tx, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let outcome = serve(worker_pipe, timeout, |pipe, events| {
                let _ = ready_tx.send(());
                action(pipe, events);
            });
            let _ = result_tx.send(outcome);
        });
        let task = Self {
            server,
            result,
            worker: Some(worker),
        };
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("production lifecycle did not start the test handler");
        task
    }

    pub fn assert_running(&self, duration: Duration) {
        assert!(matches!(
            self.result.recv_timeout(duration),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
    }

    pub fn wait(&self) -> ConnectionOutcome {
        self.result
            .recv_timeout(Duration::from_secs(2))
            .expect("production lifecycle exceeded the external test safety deadline")
            .expect("production lifecycle failed to start")
    }
}

impl Drop for LifecycleWorker {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            unsafe {
                let _ = CancelSynchronousIo(HANDLE(worker.as_raw_handle()));
                let pipe = HANDLE(self.server.as_raw_handle());
                let _ = CancelIoEx(pipe, None);
                let _ = DisconnectNamedPipe(pipe);
            }
            if let Err(payload) = worker.join() {
                if !std::thread::panicking() {
                    std::panic::resume_unwind(payload);
                }
            }
        }
    }
}
