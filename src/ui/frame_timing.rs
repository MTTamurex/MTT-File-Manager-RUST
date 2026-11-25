//! Per-second frame timing summary while the UI is busy (`MTT_TRACE_FRAME_GAPS=1`).
//!
//! It separates the three parts of a frame interval:
//! - `ui`: CPU time inside `App::ui` (our code),
//! - `cpu`: eframe's whole frame minus the time spent in `get_current_texture`
//!   and `present` (our code + tessellation + paint submission),
//! - `wait`: interval minus `cpu` (blocked on the swapchain / vsync, or idle).
//!
//! A high `wait` share while frames are slow means presentation is the
//! bottleneck (swapchain, DWM composition, cross-adapter copy), not the app.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

const SUMMARY_PERIOD: Duration = Duration::from_secs(1);
const MIN_SAMPLES: usize = 10;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("MTT_TRACE_FRAME_GAPS")
            .map(|value| value.trim() == "1")
            .unwrap_or(false)
    })
}

#[derive(Default)]
pub(crate) struct FrameTimingSummary {
    started: Option<Instant>,
    interval_ms: Vec<f32>,
    cpu_ms: Vec<f32>,
    ui_ms: Vec<f32>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Percentiles {
    pub p50: f32,
    pub p95: f32,
    pub max: f32,
}

pub(crate) fn percentiles(values: &mut [f32]) -> Option<Percentiles> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable_by(f32::total_cmp);
    let at = |q: f32| values[((values.len() - 1) as f32 * q).round() as usize];
    Some(Percentiles {
        p50: at(0.5),
        p95: at(0.95),
        max: values[values.len() - 1],
    })
}

impl FrameTimingSummary {
    /// Record one busy frame; logs and resets once per [`SUMMARY_PERIOD`].
    pub(crate) fn record(
        &mut self,
        now: Instant,
        interval_ms: f32,
        cpu_ms: Option<f32>,
        ui_ms: f32,
        context: impl FnOnce() -> String,
    ) {
        let started = *self.started.get_or_insert(now);
        self.interval_ms.push(interval_ms);
        if let Some(cpu_ms) = cpu_ms {
            self.cpu_ms.push(cpu_ms);
        }
        self.ui_ms.push(ui_ms);
        if now.saturating_duration_since(started) < SUMMARY_PERIOD {
            return;
        }
        if self.interval_ms.len() >= MIN_SAMPLES {
            self.log(context());
        }
        *self = Self::default();
    }

    /// Busy period ended: drop a partial window so idle time is not mixed in.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    fn log(&mut self, context: String) {
        let frames = self.interval_ms.len();
        let total_interval: f32 = self.interval_ms.iter().sum();
        let total_cpu: f32 = self.cpu_ms.iter().sum();
        let wait_share = if total_interval > 0.0 && !self.cpu_ms.is_empty() {
            ((total_interval - total_cpu) / total_interval).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let fps = if total_interval > 0.0 {
            frames as f32 * 1000.0 / total_interval
        } else {
            0.0
        };
        let fmt = |p: Option<Percentiles>| {
            p.map(|p| format!("{:.1}/{:.1}/{:.1}", p.p50, p.p95, p.max))
                .unwrap_or_else(|| "-".to_string())
        };
        log::info!(
            "[PERF] frame timing frames={} fps={:.0} interval_ms(p50/p95/max)={} cpu_ms={} ui_ms={} wait_share={:.0}% {}",
            frames,
            fps,
            fmt(percentiles(&mut self.interval_ms)),
            fmt(percentiles(&mut self.cpu_ms)),
            fmt(percentiles(&mut self.ui_ms)),
            wait_share * 100.0,
            context
        );
    }
}

struct Tracker {
    last_logic: Option<Instant>,
    summary: FrameTimingSummary,
}

thread_local! {
    static TRACKER: std::cell::RefCell<Tracker> = const {
        std::cell::RefCell::new(Tracker {
            last_logic: None,
            summary: FrameTimingSummary {
                started: None,
                interval_ms: Vec::new(),
                cpu_ms: Vec::new(),
                ui_ms: Vec::new(),
            },
        })
    };
}

/// Called at the start of every `App::logic` pass of the main window.
pub(crate) fn record_frame(
    app: &crate::app::ImageViewerApp,
    ctx: &eframe::egui::Context,
    frame: &eframe::Frame,
) {
    if !enabled() {
        return;
    }
    let now = Instant::now();
    let scrolling = app.thumbnail_scroll_is_settling();
    // Count only intervals the app asked to be back-to-back; an idle wait for
    // input is not a slow frame.
    let busy = scrolling || ctx.requested_repaint_last_pass();
    TRACKER.with(|tracker| {
        let mut tracker = tracker.borrow_mut();
        let previous = tracker.last_logic.replace(now);
        let Some(previous) = previous else {
            return;
        };
        if !busy {
            tracker.summary.reset();
            return;
        }
        let interval_ms = now.saturating_duration_since(previous).as_secs_f32() * 1000.0;
        let cpu_ms = frame.info().cpu_usage.map(|seconds| seconds * 1000.0);
        tracker.summary.record(
            now,
            interval_ms,
            cpu_ms,
            app.last_actual_frame_ms,
            || {
                format!(
                    "scrolling={} pending_uploads={} textures={} backend={} gpu={:?} cross_adapter={}",
                    scrolling,
                    app.cache_manager.pending_upload_set.len(),
                    app.cache_manager.texture_cache.len(),
                    app.active_gpu_backend,
                    app.active_gpu_device,
                    app.gpu_cross_adapter_display.is_some(),
                )
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_of_sorted_samples() {
        let mut values: Vec<f32> = (1..=100).map(|v| v as f32).collect();
        values.reverse();
        let p = percentiles(&mut values).unwrap();
        assert_eq!(p.p50, 51.0);
        assert_eq!(p.p95, 95.0);
        assert_eq!(p.max, 100.0);
        assert_eq!(percentiles(&mut []), None);
    }

    #[test]
    fn summary_resets_after_each_period() {
        let mut summary = FrameTimingSummary::default();
        let start = Instant::now();
        for frame in 0..30u64 {
            let now = start + Duration::from_millis(frame * 40);
            summary.record(now, 40.0, Some(5.0), 3.0, String::new);
        }
        // 30 frames at 40 ms span 1.16 s: one window was logged and cleared.
        assert!(summary.interval_ms.len() < 30);
    }
}
