//! Experimental video-only backend, isolated in a supervised same-exe process.
mod client;
mod helper;
pub(crate) mod pipeline;
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
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    loop {
        if STOPPED.load(Ordering::Acquire) || cancelled() {
            return Err(Error::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        if let Some(mut client) = service.client.try_lock_for(Duration::from_millis(20)) {
            // Includes queue wait in the deadline. Re-check shutdown after acquiring.
            if STOPPED.load(Ordering::Acquire) {
                return Err(Error::Cancelled);
            }
            return client.extract_until(path, max_side, cancelled, deadline);
        }
    }
}

pub fn shutdown() {
    STOPPED.store(true, std::sync::atomic::Ordering::Release);
    if let Some(service) = SERVICE.get() {
        service.control.shutdown();
    }
}
