use super::state::MpvState;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

/// PERF FASE 2: Starts async polling thread for offloading FFI calls from main thread
///
/// This moves the polling to a background thread, preventing main thread blocking.
/// Polls at 4 FPS (250ms) but from a separate thread, keeping UI responsive.
pub fn start_event_loop(
    mpv: Arc<mpv::Mpv>,
    state: Arc<RwLock<MpvState>>,
    running: Arc<AtomicBool>,
    ctx: egui::Context,
) -> thread::JoinHandle<()> {
    running.store(true, Ordering::Release);

    // Spawn background polling thread
    thread::spawn(move || {
        log::info!("[MpvPreview] Async polling thread started");

        loop {
            // Check shutdown flag
            if !running.load(Ordering::Acquire) {
                log::info!("[MpvPreview] Async polling thread stopping...");
                break;
            }

            // Poll properties (moved to background thread - zero impact on main thread!)
            let mut state_updated = false;

            // Poll time position
            if let Ok(pos) = mpv.get_property::<f64>("time-pos") {
                if let Ok(mut s) = state.write() {
                    s.current_time = pos;
                    state_updated = true;
                }
            }

            // Poll pause state
            if let Ok(paused) = mpv.get_property::<bool>("pause") {
                if let Ok(mut s) = state.write() {
                    s.is_playing = !paused;
                    state_updated = true;
                }
            }

            // Poll volume
            if let Ok(vol) = mpv.get_property::<f64>("volume") {
                if let Ok(mut s) = state.write() {
                    s.volume = (vol / 100.0).clamp(0.0, 1.0) as f32;
                    state_updated = true;
                }
            }

            // Poll mute state
            if let Ok(muted) = mpv.get_property::<bool>("mute") {
                if let Ok(mut s) = state.write() {
                    s.is_muted = muted;
                    state_updated = true;
                }
            }

            // Poll duration (only once until it's available)
            if let Ok(dur) = mpv.get_property::<f64>("duration") {
                if let Ok(mut s) = state.write() {
                    if s.duration == 0.0 || s.duration != dur {
                        s.duration = dur;
                        state_updated = true;
                    }
                }
            }

            // Request UI repaint only if state changed
            if state_updated {
                ctx.request_repaint();
            }

            // Sleep 250ms between polls (4 FPS)
            thread::sleep(Duration::from_millis(250));
        }

        log::info!("[MpvPreview] Async polling thread exited");
    })
}

/// Stop the event loop gracefully
pub fn stop_event_loop(running: Arc<AtomicBool>, handle: Option<thread::JoinHandle<()>>) {
    if running.load(Ordering::Relaxed) {
        log::info!("[MpvPreview] Shutting down event loop thread...");

        // Signal thread to stop
        running.store(false, Ordering::Release);

        // Wait for thread to exit (with timeout to prevent hanging)
        if let Some(handle) = handle {
            // Give thread up to 2 seconds to exit gracefully
            let start = std::time::Instant::now();
            while !handle.is_finished() && start.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(50));
            }

            if handle.is_finished() {
                match handle.join() {
                    Ok(_) => log::info!("[MpvPreview] Event loop thread joined successfully"),
                    Err(_) => log::warn!("[MpvPreview] Event loop thread panicked"),
                }
            } else {
                // Intentionally avoid unconditional join here to preserve timeout semantics.
                log::warn!(
                    "[MpvPreview] Event loop thread did not finish within timeout; skipping blocking join"
                );
            }
        }
    }
}
