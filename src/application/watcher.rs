//! File system watcher state management
//! Follows .cursorrules: single responsibility, < 300 lines

use std::time::Instant;

/// File system watcher state
#[derive(Clone, Debug)]
pub struct WatcherState {
    pub last_auto_reload: Instant,
    pub pending_auto_reload: bool,
}

impl Default for WatcherState {
    fn default() -> Self {
        Self {
            last_auto_reload: Instant::now(),
            pending_auto_reload: false,
        }
    }
}

impl WatcherState {
    /// Creates a new watcher state
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if auto-reload should be triggered
    pub fn should_auto_reload(&self, debounce_ms: u64) -> bool {
        self.pending_auto_reload
            && self.last_auto_reload.elapsed().as_millis() >= debounce_ms as u128
    }

    /// Requests an auto-reload
    pub fn request_auto_reload(&mut self) {
        // L-10: only start the debounce timer on the FIRST event in a burst.
        // Resetting last_auto_reload on every event would defer the reload indefinitely.
        if !self.pending_auto_reload {
            self.last_auto_reload = Instant::now();
        }
        self.pending_auto_reload = true;
    }

    /// Completes an auto-reload
    pub fn complete_auto_reload(&mut self) {
        self.pending_auto_reload = false;
    }

    /// Resets the watcher state
    pub fn reset(&mut self) {
        self.last_auto_reload = Instant::now();
        self.pending_auto_reload = false;
    }
}
