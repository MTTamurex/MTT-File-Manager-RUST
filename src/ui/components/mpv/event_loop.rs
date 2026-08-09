use super::state::{FileLoadGate, MpvState, PendingFileLoad, PendingSeekState};
use eframe::egui;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const SEEK_SETTLE_TOLERANCE_SECS: f64 = 0.35;
const SEEK_PENDING_TIMEOUT: Duration = Duration::from_millis(1200);
const EVENT_FALLBACK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
enum FallbackLoadAction {
    Wait,
    Complete,
    Cancel,
}

pub struct EventLoopShared {
    state: Arc<RwLock<MpvState>>,
    running: Arc<AtomicBool>,
    tracks_need_query: Arc<AtomicU64>,
    file_load_gate: Arc<FileLoadGate>,
    pending_seek: Arc<RwLock<Option<PendingSeekState>>>,
    ctx: egui::Context,
}

impl EventLoopShared {
    pub fn new(
        state: Arc<RwLock<MpvState>>,
        running: Arc<AtomicBool>,
        tracks_need_query: Arc<AtomicU64>,
        file_load_gate: Arc<FileLoadGate>,
        pending_seek: Arc<RwLock<Option<PendingSeekState>>>,
        ctx: egui::Context,
    ) -> Self {
        Self {
            state,
            running,
            tracks_need_query,
            file_load_gate,
            pending_seek,
            ctx,
        }
    }
}

fn normalized_mpv_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

fn loaded_path_matches(load: &PendingFileLoad, current_path: Option<&str>) -> bool {
    current_path
        .is_some_and(|path| normalized_mpv_path(path) == normalized_mpv_path(&load.expected_path))
}

fn fallback_load_action(
    pending: &PendingFileLoad,
    current_path: Option<&str>,
    now: Instant,
) -> FallbackLoadAction {
    if loaded_path_matches(pending, current_path) {
        FallbackLoadAction::Complete
    } else if now.saturating_duration_since(pending.requested_at) >= EVENT_FALLBACK_TIMEOUT {
        FallbackLoadAction::Cancel
    } else {
        FallbackLoadAction::Wait
    }
}

/// PERF FASE 2: Starts async polling thread for offloading FFI calls from main thread
///
/// This moves the polling to a background thread, preventing main thread blocking.
/// Polls at 4 FPS (250ms) but from a separate thread, keeping UI responsive.
pub fn start_event_loop(
    mpv: Arc<mpv::Mpv>,
    mut event_client: Option<mpv::Mpv>,
    shared: EventLoopShared,
) -> thread::JoinHandle<()> {
    let EventLoopShared {
        state,
        running,
        tracks_need_query,
        file_load_gate,
        pending_seek,
        ctx,
    } = shared;
    running.store(true, Ordering::Release);

    // Spawn background polling thread
    thread::spawn(move || {
        log::info!("[MpvPreview] Async polling thread started");

        let mut last_interlace_check = Instant::now();
        // OPT-5: Tiered polling counters (each tick = 250ms)
        let mut tick_count: u32 = 0;
        let mut active_load: Option<PendingFileLoad> = None;
        let mut use_event_fallback = event_client.is_none();
        const MEDIUM_TIER_TICKS: u32 = 4; // ~1s for volume/mute/duration
        const SLOW_TIER_TICKS: u32 = 8; // ~2s for fullscreen/aspect

        loop {
            // Check shutdown flag
            if !running.load(Ordering::Acquire) {
                log::info!("[MpvPreview] Async polling thread stopping...");
                break;
            }

            if let Some(client) = event_client.as_mut() {
                loop {
                    match client.wait_event(0.0) {
                        Some(Ok(mpv::events::Event::StartFile)) => {
                            active_load = file_load_gate.take_start_file_registration();
                        }
                        Some(Ok(mpv::events::Event::FileLoaded)) => {
                            if let Some(load) = active_load.take() {
                                let current_path = mpv.get_property::<String>("path").ok();
                                if loaded_path_matches(&load, current_path.as_deref()) {
                                    if file_load_gate.note_file_loaded(load.generation) {
                                        log::debug!(
                                            "[MpvPreview] FileLoaded released generation {}",
                                            load.generation
                                        );
                                        ctx.request_repaint();
                                    }
                                } else if file_load_gate.loading_generation() == load.generation {
                                    log::warn!(
                                        "[MpvPreview] Ignored FileLoaded with mismatched path for generation {}",
                                        load.generation
                                    );
                                    use_event_fallback = true;
                                }
                            }
                        }
                        Some(Ok(mpv::events::Event::EndFile(_))) => {
                            if let Some(load) = active_load.take() {
                                let _ = file_load_gate.cancel_load(load.generation);
                            }
                        }
                        Some(Ok(mpv::events::Event::QueueOverflow)) => {
                            log::warn!(
                                "[MpvPreview] Event queue overflow; enabling verified-path fallback"
                            );
                            use_event_fallback = true;
                        }
                        Some(Ok(mpv::events::Event::Shutdown)) => {
                            running.store(false, Ordering::Release);
                            break;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(error)) => {
                            log::warn!(
                                "[MpvPreview] Event client error; enabling verified-path fallback: {:?}",
                                error
                            );
                            use_event_fallback = true;
                        }
                        None => break,
                    }
                }
            }

            if use_event_fallback
                && file_load_gate.is_parked()
                && !file_load_gate.is_command_pending()
            {
                if let Some(pending) = file_load_gate.pending_load() {
                    let current_path = mpv.get_property::<String>("path").ok();
                    match fallback_load_action(&pending, current_path.as_deref(), Instant::now()) {
                        FallbackLoadAction::Complete => {
                            if file_load_gate.note_file_loaded(pending.generation) {
                                log::warn!(
                                    "[MpvPreview] Verified-path fallback released generation {}",
                                    pending.generation
                                );
                                ctx.request_repaint();
                            }
                        }
                        FallbackLoadAction::Cancel => {
                            log::warn!(
                                "[MpvPreview] Event fallback timed out for generation {}; cancelling load",
                                pending.generation
                            );
                            let empty: [&str; 0] = [];
                            let _ = mpv.command("stop", &empty);
                            let _ = file_load_gate.cancel_load(pending.generation);
                            ctx.request_repaint();
                        }
                        FallbackLoadAction::Wait => {}
                    }
                }
            }

            if file_load_gate.is_command_pending() || file_load_gate.is_parked() {
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            let observed_generation = file_load_gate.generation();

            let mut state_updated = false;
            // OPT-1: Track whether a significant state change occurred
            let mut significant_change = false;
            let current_duration: f64;

            // --- Fast tier (every 250ms): time-pos + pause ---
            // Skip time-pos writes while a new file is loading to prevent
            // stale values from the old file overwriting the reset.
            if file_load_gate.loading_generation() != observed_generation {
                if let Ok(pos) = mpv.get_property::<f64>("time-pos") {
                    if !file_load_gate.result_is_current(observed_generation) {
                        continue;
                    }
                    let mut allow_polled_position = true;

                    if let Ok(mut pending) = pending_seek.write() {
                        if file_load_gate.result_is_current(observed_generation) {
                            if let Some(pending_seek_state) = pending.as_ref() {
                                let seek_has_settled = (pos - pending_seek_state.target_time).abs()
                                    <= SEEK_SETTLE_TOLERANCE_SECS;
                                let seek_wait_expired = pending_seek_state.requested_at.elapsed()
                                    >= SEEK_PENDING_TIMEOUT;

                                if seek_has_settled || seek_wait_expired {
                                    *pending = None;
                                } else {
                                    allow_polled_position = false;
                                }
                            }
                        }
                    }

                    if allow_polled_position {
                        if let Ok(mut s) = state.write() {
                            if file_load_gate.result_is_current(observed_generation)
                                && (s.current_time - pos).abs() > 0.001
                            {
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
                    if file_load_gate.result_is_current(observed_generation)
                        && s.is_playing != new_playing
                    {
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
                    if file_load_gate.result_is_current(observed_generation) && s.fullscreen != fs {
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
                        if file_load_gate.result_is_current(observed_generation)
                            && (s.volume - new_vol).abs() > 0.001
                        {
                            s.volume = new_vol;
                            state_updated = true;
                        }
                    }
                }

                if let Ok(muted) = mpv.get_property::<bool>("mute") {
                    if let Ok(mut s) = state.write() {
                        if file_load_gate.result_is_current(observed_generation)
                            && s.is_muted != muted
                        {
                            s.is_muted = muted;
                            state_updated = true;
                            significant_change = true;
                        }
                    }
                }

                if let Ok(dur) = mpv.get_property::<f64>("duration") {
                    current_duration = dur;
                    if let Ok(mut s) = state.write() {
                        if file_load_gate.result_is_current(observed_generation)
                            && (s.duration == 0.0 || (s.duration - dur).abs() > 0.01)
                        {
                            s.duration = dur;
                            state_updated = true;
                            significant_change = true;
                        }
                    }
                } else {
                    current_duration = state.read().map(|s| s.duration).unwrap_or(0.0);
                }
            } else {
                // Need current_duration for track/interlace checks below
                current_duration = state.read().map(|s| s.duration).unwrap_or(0.0);
            }

            // --- Slow tier (~2s): aspect ---
            if tick_count.is_multiple_of(SLOW_TIER_TICKS) && current_duration > 0.0 {
                let aspect = super::playback::get_video_aspect(&mpv);
                if let Ok(mut s) = state.write() {
                    if file_load_gate.result_is_current(observed_generation)
                        && s.video_aspect != aspect
                    {
                        s.video_aspect = aspect;
                        state_updated = true;
                        significant_change = true;
                    }
                }
            }

            // Query tracks when signaled and file is ready (PERF: moved from render thread)
            if observed_generation != 0
                && tracks_need_query.load(Ordering::Acquire) == observed_generation
                && current_duration > 0.0
            {
                let (audio, subs) = super::playback::query_tracks(&mpv);
                if let Ok(mut s) = state.write() {
                    if file_load_gate.result_is_current(observed_generation) {
                        s.audio_tracks = audio;
                        s.subtitle_tracks = subs;
                        s.tracks_ready = true;
                        state_updated = true;
                        significant_change = true;
                    }
                }
                let _ = tracks_need_query.compare_exchange(
                    observed_generation,
                    0,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }

            // Detect interlaced status every ~2s (moved from 500ms)
            if current_duration > 0.0 && last_interlace_check.elapsed() >= Duration::from_secs(2) {
                let interlaced = super::playback::detect_interlaced(&mpv);
                if let Ok(mut s) = state.write() {
                    if file_load_gate.result_is_current(observed_generation)
                        && s.interlaced != interlaced
                    {
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

pub fn signal_stop(running: &AtomicBool) {
    running.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::{fallback_load_action, FallbackLoadAction, EVENT_FALLBACK_TIMEOUT};
    use crate::ui::components::mpv::state::PendingFileLoad;
    use std::time::{Duration, Instant};

    #[test]
    fn fallback_completes_only_for_the_expected_path() {
        let now = Instant::now();
        let pending = PendingFileLoad {
            generation: 4,
            expected_path: "C:/media/b.mp4".into(),
            requested_at: now,
        };

        assert_eq!(
            fallback_load_action(&pending, Some(r"c:\media\b.mp4"), now),
            FallbackLoadAction::Complete
        );
        assert_eq!(
            fallback_load_action(&pending, Some("C:/media/a.mp4"), now),
            FallbackLoadAction::Wait
        );
    }

    #[test]
    fn fallback_cancels_after_timeout_instead_of_parking_forever() {
        let now = Instant::now();
        let pending = PendingFileLoad {
            generation: 7,
            expected_path: "B.mp4".into(),
            requested_at: now,
        };

        assert_eq!(
            fallback_load_action(
                &pending,
                None,
                now + EVENT_FALLBACK_TIMEOUT - Duration::from_millis(1)
            ),
            FallbackLoadAction::Wait
        );
        assert_eq!(
            fallback_load_action(&pending, None, now + EVENT_FALLBACK_TIMEOUT),
            FallbackLoadAction::Cancel
        );
    }
}
