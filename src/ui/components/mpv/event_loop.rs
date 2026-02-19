use super::state::MpvState;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

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
                    if let Ok(mut s) = state.write() {
                        s.current_time = pos;
                        state_updated = true;
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

            // --- Medium tier (~1s): volume, mute, duration ---
            if tick_count % MEDIUM_TIER_TICKS == 0 {
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

            // --- Slow tier (~2s): fullscreen, aspect ---
            if tick_count % SLOW_TIER_TICKS == 0 {
                if let Ok(fs) = mpv.get_property::<bool>("fullscreen") {
                    if let Ok(mut s) = state.write() {
                        if s.fullscreen != fs {
                            s.fullscreen = fs;
                            state_updated = true;
                            significant_change = true;
                        }
                    }
                }

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
