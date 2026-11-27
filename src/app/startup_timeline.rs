//! Process-relative timestamps for the main-window startup timeline logs.

use std::sync::OnceLock;
use std::time::Instant;

static PROCESS_START: OnceLock<Instant> = OnceLock::new();

/// Records the startup origin. Only the first call has an effect.
pub fn start() {
    let _ = PROCESS_START.get_or_init(Instant::now);
}

/// Milliseconds since [`start`], or 0 when it was never called.
pub fn elapsed_ms() -> u128 {
    PROCESS_START
        .get()
        .map_or(0, |start| start.elapsed().as_millis())
}
