//! Video-only backend, isolated in a supervised same-executable process.
mod client;
mod helper;
mod process;
mod protocol;
#[cfg(test)]
mod tests;

pub use client::Extractor;
pub use helper::run_helper;
pub use protocol::{Failure, Frame};

use parking_lot::Mutex;
use std::{
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub const HELPER_FLAG: &str = "--internal-mpv-thumbnail-helper";
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    Timeout,
    QueueTimeout,
    Transport,
    Unavailable,
    Decode,
    Helper(Failure),
}

struct Service {
    client: Mutex<Extractor>,
    control: std::sync::Arc<process::Control>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();
static STOPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn is_stopped() -> bool {
    STOPPED.load(std::sync::atomic::Ordering::Acquire)
}

pub fn extract(path: &Path, max_side: u32, cancelled: &dyn Fn() -> bool) -> Result<Frame, Error> {
    use std::sync::atomic::Ordering;
    if STOPPED.load(Ordering::Acquire) {
        return Err(Error::Cancelled);
    }
    let service = SERVICE.get_or_init(|| {
        let client = Extractor::new(std::env::current_exe().unwrap_or_default());
        Service {
            control: client.control.clone(),
            client: Mutex::new(client),
        }
    });
    service.extract_until(
        path,
        max_side,
        &|| STOPPED.load(Ordering::Acquire) || cancelled(),
        Instant::now() + REQUEST_TIMEOUT,
    )
}

impl Service {
    fn extract_until(
        &self,
        path: &Path,
        max_side: u32,
        cancelled: &dyn Fn() -> bool,
        deadline: Instant,
    ) -> Result<Frame, Error> {
        loop {
            if self.control.stopped() || cancelled() {
                return Err(Error::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(Error::QueueTimeout);
            }
            if let Some(mut client) = self.client.try_lock_for(Duration::from_millis(20)) {
                // Includes queue wait in the deadline. Do not touch a healthy helper
                // if the request expired/cancelled before obtaining its slot.
                if self.control.stopped() || cancelled() {
                    return Err(Error::Cancelled);
                }
                if Instant::now() >= deadline {
                    return Err(Error::QueueTimeout);
                }
                return client.extract_until(path, max_side, cancelled, deadline);
            }
        }
    }
}

pub fn shutdown() {
    STOPPED.store(true, std::sync::atomic::Ordering::Release);
    if let Some(service) = SERVICE.get() {
        service.control.shutdown();
    }
}
