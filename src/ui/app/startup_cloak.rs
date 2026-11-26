//! Hides the main window's startup geometry changes.
//!
//! Applying `Maximized(true)` to the hidden startup window makes Windows show
//! it (`ShowWindow(SW_MAXIMIZE)`) at its 800x600 creation size and animate the
//! maximize before it is hidden again and revealed. The window is therefore
//! DWM-cloaked before its first geometry command and uncloaked only after a
//! frame has been painted while visible at the final geometry.

use crate::app::ImageViewerApp;
use crate::infrastructure::windows::window_cloak::set_window_cloaked;
use eframe::egui;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;

/// Upper bound so a missed state transition can never leave the window invisible.
const MAX_CLOAK_DURATION: Duration = Duration::from_millis(1500);

/// Ready frames needed before uncloaking: the first one is painted at the end
/// of its pass, so the window is uncloaked on the following pass.
const READY_FRAMES_TO_REVEAL: u8 = 2;

pub(crate) struct StartupCloak {
    hwnd: HWND,
    since: Instant,
    expect_maximized: bool,
    ready_frames: u8,
}

impl StartupCloak {
    /// Cloaks `hwnd`. Returns `None` when DWM rejects cloaking.
    pub(crate) fn begin(hwnd: HWND, expect_maximized: bool) -> Option<Self> {
        if !set_window_cloaked(hwnd, true) {
            log::warn!("[STARTUP] DWM cloak unavailable; startup geometry changes may be visible");
            return None;
        }
        Some(Self {
            hwnd,
            since: Instant::now(),
            expect_maximized,
            ready_frames: 0,
        })
    }
}

/// Uncloaks the main window once a frame was painted at the final geometry.
pub fn release_startup_cloak_when_painted(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(cloak) = app.startup_cloak.as_mut() else {
        return;
    };

    let ready = ctx.input(|i| {
        let viewport = i.viewport();
        viewport.visible() == Some(true)
            && (!cloak.expect_maximized || viewport.maximized == Some(true))
    });
    if ready {
        cloak.ready_frames = cloak.ready_frames.saturating_add(1);
    }

    let elapsed = cloak.since.elapsed();
    if !should_release(cloak.ready_frames, elapsed) {
        ctx.request_repaint();
        return;
    }

    if !set_window_cloaked(cloak.hwnd, false) {
        log::warn!("[STARTUP] Failed to uncloak main window");
    }
    log::info!(
        "[STARTUP] main window uncloaked ready_frames={} elapsed_ms={}",
        cloak.ready_frames,
        elapsed.as_millis()
    );
    app.startup_cloak = None;
}

fn should_release(ready_frames: u8, elapsed: Duration) -> bool {
    ready_frames >= READY_FRAMES_TO_REVEAL || elapsed >= MAX_CLOAK_DURATION
}

#[cfg(test)]
mod tests {
    use super::{should_release, MAX_CLOAK_DURATION};
    use std::time::Duration;

    #[test]
    fn waits_for_a_painted_frame_at_final_geometry() {
        assert!(!should_release(0, Duration::ZERO));
        assert!(!should_release(1, Duration::from_millis(10)));
        assert!(should_release(2, Duration::from_millis(10)));
    }

    #[test]
    fn timeout_always_releases() {
        assert!(should_release(0, MAX_CLOAK_DURATION));
    }
}
