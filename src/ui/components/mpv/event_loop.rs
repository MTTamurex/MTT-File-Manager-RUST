use super::state::{MpvState, PendingSeekState};
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const SEEK_SETTLE_TOLERANCE_SECS: f64 = 0.35;
const SEEK_PENDING_TIMEOUT: Duration = Duration::from_millis(1200);

/// PERF FASE 2: Starts async polling thread for offloading FFI calls from main thread
///
/// This moves the polling to a background thread, preventing main thread blocking.
/// Polls at 4 FPS (250ms) but from a separate thread, keeping UI responsive.
pub fn start_event_loop(
    mpv: Arc<mpv::Mpv>,
    state: Arc<RwLock<MpvState>>,
    running: Arc<AtomicBool>,
    tracks_need_query: Arc<AtomicBool>,
    file_loading: Arc<AtomicBool>,
    pending_seek: Arc<RwLock<Option<PendingSeekState>>>,
    ctx: egui::Context,
) -> thread::JoinHandle<()> {
    running.store(true, Ordering::Release);

    // Spawn background polling thread
    thread::spawn(move || {
        log::info!("[MpvPreview] Async polling thread started");

        let mut last_interlace_check = Instant::now();
        // OPT-5: Tiered polling counters (each tick = 250ms)
        let mut tick_count: u32 = 0;
        const MEDIUM_TIER_TICKS: u32 = 4; // ~1s for volume/mute/duration
        const SLOW_TIER_TICKS: u32 = 8; // ~2s for fullscreen/aspect

        loop {
            // Check shutdown flag
            if !running.load(Ordering::Acquire) {
                log::info!("[MpvPreview] Async polling thread stopping...");
                break;
            }

            let mut state_updated = false;
            // OPT-1: Track whether a significant state change occurred
            let mut significant_change = false;
            let current_duration: f64;

            // --- Fast tier (every 250ms): time-pos + pause ---
            // Skip time-pos writes while a new file is loading to prevent
            // stale values from the old file overwriting the reset.
            if !file_loading.load(Ordering::Acquire) {
                if let Ok(pos) = mpv.get_property::<f64>("time-pos") {
                    let mut allow_polled_position = true;

                    if let Ok(mut pending) = pending_seek.write() {
                        if let Some(pending_seek_state) = pending.as_ref() {
                            let seek_has_settled = (pos - pending_seek_state.target_time).abs()
                                <= SEEK_SETTLE_TOLERANCE_SECS;
                            let seek_wait_expired =
                                pending_seek_state.requested_at.elapsed() >= SEEK_PENDING_TIMEOUT;

                            if seek_has_settled || seek_wait_expired {
                                *pending = None;
                            } else {
                                allow_polled_position = false;
                            }
                        }
                    }

                    if allow_polled_position {
                        if let Ok(mut s) = state.write() {
                            if (s.current_time - pos).abs() > 0.001 {
                                s.current_time = pos;
                                state_updated = true;
                            }
                        }
                    }
                }
            }

            // Pause is fast tier: critical for OSC suppression and play button
            if let Ok(paused) = mpv.get_property::<bool>("pause") {
                if let Ok(mut s) = state.write() {
                    let new_playing = !paused;
                    if s.is_playing != new_playing {
                        s.is_playing = new_playing;
                        state_updated = true;
                        significant_change = true;
                    }
                }
            }

            // Fullscreen is fast tier: critical for OSC button responsiveness.
            // Without this, the OSC fullscreen button has a ~2s delay because
            // the slow tier only polls every 2 seconds. Boolean read = negligible cost.
            if let Ok(fs) = mpv.get_property::<bool>("fullscreen") {
                if let Ok(mut s) = state.write() {
                    if s.fullscreen != fs {
                        s.fullscreen = fs;
                        state_updated = true;
                        significant_change = true;
                    }
                }
            }

            // --- Medium tier (~1s): volume, mute, duration ---
            if tick_count.is_multiple_of(MEDIUM_TIER_TICKS) {
                if let Ok(vol) = mpv.get_property::<f64>("volume") {
                    if let Ok(mut s) = state.write() {
                        let new_vol = (vol / 100.0).clamp(0.0, 1.0) as f32;
                        if (s.volume - new_vol).abs() > 0.001 {
                            s.volume = new_vol;
                            state_updated = true;
                        }
                    }
                }

                if let Ok(muted) = mpv.get_property::<bool>("mute") {
                    if let Ok(mut s) = state.write() {
                        if s.is_muted != muted {
                            s.is_muted = muted;
                            state_updated = true;
                            significant_change = true;
                        }
                    }
                }

                if let Ok(dur) = mpv.get_property::<f64>("duration") {
                    current_duration = dur;
                    if let Ok(mut s) = state.write() {
                        if s.duration == 0.0 || (s.duration - dur).abs() > 0.01 {
                            s.duration = dur;
                            state_updated = true;
                            significant_change = true;
                        }
                    }
                    // New file is ready — unblock time-pos writes.
                    // Release even when dur == 0.0 (streams/corrupt media may never
                    // report a positive duration, which would keep file_loading true
                    // forever and permanently suppress time-pos polling).
                    if file_loading.load(Ordering::Acquire) {
                        file_loading.store(false, Ordering::Release);
                    }
                } else {
                    current_duration = state.read().map(|s| s.duration).unwrap_or(0.0);
                }
            } else {
                // Need current_duration for track/interlace checks below
                current_duration = state.read().map(|s| s.duration).unwrap_or(0.0);
            }

            // --- Slow tier (~2s): aspect ---
            if tick_count.is_multiple_of(SLOW_TIER_TICKS) {
                if current_duration > 0.0 {
                    let aspect = super::playback::get_video_aspect(&mpv);
                    if let Ok(mut s) = state.write() {
                        if s.video_aspect != aspect {
                            s.video_aspect = aspect;
                            state_updated = true;
                            significant_change = true;
                        }
                    }
                }
            }

            // Query tracks when signaled and file is ready (PERF: moved from render thread)
            if tracks_need_query.load(Ordering::Acquire) && current_duration > 0.0 {
                let (audio, subs) = super::playback::query_tracks(&mpv);
                if let Ok(mut s) = state.write() {
                    s.audio_tracks = audio;
                    s.subtitle_tracks = subs;
                    s.tracks_ready = true;
                    state_updated = true;
                    significant_change = true;
                }
                tracks_need_query.store(false, Ordering::Release);
            }

            // Detect interlaced status every ~2s (moved from 500ms)
            if current_duration > 0.0 && last_interlace_check.elapsed() >= Duration::from_secs(2) {
                let interlaced = super::playback::detect_interlaced(&mpv);
                if let Ok(mut s) = state.write() {
                    if s.interlaced != interlaced {
                        s.interlaced = interlaced;
                        state_updated = true;
                        significant_change = true;
                    }
                }
                last_interlace_check = Instant::now();
            }

            // OPT-1: Selective repaint — immediate for significant changes,
            // delayed for incremental time-pos updates.
            if significant_change {
                ctx.request_repaint();
            } else if state_updated {
                ctx.request_repaint_after(Duration::from_millis(500));
            }

            tick_count = tick_count.wrapping_add(1);

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

        // Wait briefly for the thread to exit.  The event loop polls
        // `running` every ~250 ms, so 300 ms is enough for one iteration.
        // Don't wait longer — process::exit will clean up regardless.
        if let Some(handle) = handle {
            let start = std::time::Instant::now();
            while !handle.is_finished() && start.elapsed() < Duration::from_millis(300) {
                std::thread::sleep(Duration::from_millis(10));
            }

            if handle.is_finished() {
                match handle.join() {
                    Ok(_) => log::info!("[MpvPreview] Event loop thread joined successfully"),
                    Err(_) => log::warn!("[MpvPreview] Event loop thread panicked"),
                }
            } else {
                log::info!(
                    "[MpvPreview] Event loop thread still running; process::exit will clean up"
                );
            }
        }
    }
}
