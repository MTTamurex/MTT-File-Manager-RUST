//! One native listener for the primary UI context in this process.
//! Viewer windows use separate processes; the main app exits the process on close.
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use super::LISTENER_FALLBACK_INTERVAL;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static STARTUP: Mutex<Startup> = Mutex::new(Startup {
    started: false,
    last_attempt: None,
});

struct Startup {
    started: bool,
    last_attempt: Option<Instant>,
}

impl Startup {
    fn begin_attempt(&mut self, now: Instant) -> bool {
        if self.started
            || self.last_attempt.is_some_and(|last| {
                now.saturating_duration_since(last) < LISTENER_FALLBACK_INTERVAL
            })
        {
            return false;
        }
        self.last_attempt = Some(now);
        true
    }
}

pub(super) fn is_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub(super) fn ensure_started(ctx: &egui::Context) {
    let mut startup = STARTUP.lock().unwrap_or_else(|error| error.into_inner());
    if !startup.begin_attempt(Instant::now()) {
        return;
    }
    let repaint = ctx.clone();
    match std::thread::Builder::new()
        .name("clipboard-change-listener".into())
        .spawn(move || loop {
            // A failure must never leave ACTIVE stuck at true. The UI keeps
            // its slow fallback while this thread retries native registration.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                receive_changes(&repaint)
            }));
            ACTIVE.store(false, Ordering::Release);
            repaint.request_repaint_of(egui::ViewportId::ROOT);
            if result.is_err() {
                log::warn!("[Clipboard] Change listener panicked; retrying");
            }
            std::thread::sleep(LISTENER_FALLBACK_INTERVAL);
        }) {
        Ok(_) => startup.started = true,
        Err(error) => log::debug!("[Clipboard] Failed to start change listener: {error}"),
    }
}

fn receive_changes(repaint: &egui::Context) {
    let mut listener = match clipboard_win::Monitor::new() {
        Ok(listener) => listener,
        Err(error) => {
            log::debug!("[Clipboard] Change listener unavailable: {error:?}");
            return;
        }
    };
    ACTIVE.store(true, Ordering::Release);
    // Recheck the sequence after registration to cover changes during startup.
    repaint.request_repaint_of(egui::ViewportId::ROOT);
    loop {
        match listener.recv() {
            Ok(true) => repaint.request_repaint_of(egui::ViewportId::ROOT),
            Ok(false) => return,
            Err(error) => {
                log::debug!("[Clipboard] Change listener stopped: {error:?}");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_thread_creation_can_retry_without_spinning() {
        let mut startup = Startup {
            started: false,
            last_attempt: None,
        };
        let now = Instant::now();
        assert!(startup.begin_attempt(now));
        assert!(!startup.begin_attempt(now));
        assert!(startup.begin_attempt(now + LISTENER_FALLBACK_INTERVAL));
        startup.started = true;
        assert!(!startup.begin_attempt(now + LISTENER_FALLBACK_INTERVAL * 2));
    }
}
