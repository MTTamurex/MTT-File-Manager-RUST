use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mtt_search_protocol::SearchResponse;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Pipes::DisconnectNamedPipe;

use super::{connection_lifecycle, pipe_io};

pub(super) struct RejectionPool {
    active: Arc<AtomicU32>,
    limit: u32,
    timeout: Duration,
}

struct Permit(Arc<AtomicU32>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RejectionStatus {
    Scheduled,
    DroppedAtCapacity,
}

impl RejectionPool {
    pub fn new(limit: u32, timeout: Duration) -> Self {
        Self {
            active: Arc::new(AtomicU32::new(0)),
            limit,
            timeout,
        }
    }

    pub fn active_count(&self) -> u32 {
        self.active.load(Ordering::Acquire)
    }

    /// Never write or flush on the accept thread. Rejections have independent
    /// bounded capacity so an overload cannot spawn an unbounded set of workers.
    pub fn reject(
        &self,
        pipe: OwnedHandle,
        response: SearchResponse,
    ) -> Result<RejectionStatus, String> {
        let previous = self.active.fetch_add(1, Ordering::AcqRel);
        if previous >= self.limit {
            self.active.fetch_sub(1, Ordering::Release);
            unsafe {
                let _ = DisconnectNamedPipe(HANDLE(pipe.as_raw_handle()));
            }
            // Extreme overload: no error-reply delivery guarantee. Closing is
            // preferable to blocking admission or creating unlimited threads.
            return Ok(RejectionStatus::DroppedAtCapacity);
        }
        let permit = Permit(Arc::clone(&self.active));
        let timeout = self.timeout;
        std::thread::Builder::new()
            .name("search-ipc-rejection".into())
            .spawn(move || {
                let _permit = permit;
                if let Err(error) = connection_lifecycle::serve(pipe, timeout, |pipe, _| {
                    let _ = pipe_io::send_response(pipe, &response);
                }) {
                    eprintln!("[IPC] Rejection lifecycle startup failed: {error}");
                }
            })
            .map_err(|error| format!("Cannot start IPC rejection worker: {error}"))?;
        Ok(RejectionStatus::Scheduled)
    }
}
