//! Thumbnail policy, kept separate from the session's FFI/event lifecycle.
use crate::{
    capture::Session,
    pixels::Screenshot,
    quality::{self, Quality},
    selection,
};
use anyhow::{ensure, Context, Result};
use image::RgbaImage;
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub struct Capture {
    pub image: RgbaImage,
    pub details: Value,
}

#[derive(Clone, Copy)]
enum Strategy {
    Adaptive,
    Explicit(f64),
    Legacy,
}

impl Strategy {
    fn name(self) -> &'static str {
        match self {
            Self::Adaptive => "adaptive-v2",
            Self::Explicit(_) => "explicit",
            Self::Legacy => "legacy-v1",
        }
    }
}

struct Candidate {
    screenshot: Screenshot,
    quality: Quality,
    requested: f64,
    actual: Option<f64>,
    seek_ms: f64,
    capture_ms: f64,
    exact: bool,
    corrected: bool,
}

impl Session<'_> {
    /// Automatic selection probes up to three dispersed positions. An explicit
    /// timestamp remains an exact single-frame request (no quality substitution).
    pub fn capture(
        &mut self,
        path: &Path,
        at: Option<f64>,
        max_side: u32,
        timeout: Duration,
    ) -> Result<Capture> {
        self.capture_policy(
            path,
            at.map_or(Strategy::Adaptive, Strategy::Explicit),
            max_side,
            timeout,
            true,
        )
    }

    /// Baseline for A/B measurement only; never used by the application helper.
    pub fn capture_legacy(
        &mut self,
        path: &Path,
        max_side: u32,
        timeout: Duration,
    ) -> Result<Capture> {
        self.capture_policy(path, Strategy::Legacy, max_side, timeout, false)
    }

    #[cfg(test)]
    pub(crate) fn capture_adaptive_unscaled(
        &mut self,
        path: &Path,
        max_side: u32,
        timeout: Duration,
    ) -> Result<Capture> {
        self.capture_policy(path, Strategy::Adaptive, max_side, timeout, false)
    }

    fn capture_policy(
        &mut self,
        path: &Path,
        strategy: Strategy,
        max_side: u32,
        timeout: Duration,
        native_scale: bool,
    ) -> Result<Capture> {
        ensure!(
            (1..=4096).contains(&max_side),
            "Invalid thumbnail target size"
        );
        if let Strategy::Explicit(at) = strategy {
            ensure!(at.is_finite() && at >= 0.0, "Invalid timestamp");
        }
        let start = Instant::now();
        let deadline = start + timeout;
        // Leave room to release the file after a soft seek/capture deadline.
        let work_deadline = deadline
            .checked_sub(Duration::from_millis(200))
            .filter(|&t| t > start)
            .unwrap_or(deadline);
        self.stop(deadline)?;
        self.scaling(native_scale.then_some(max_side), work_deadline)?;
        if matches!(strategy, Strategy::Adaptive)
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv"))
        {
            let cover = self.embedded_cover(path, max_side, work_deadline);
            if let Ok(Some(screenshot)) = cover {
                let cover_probe_ms = start.elapsed().as_secs_f64() * 1000.0;
                let stop_start = Instant::now();
                self.stop(deadline)?;
                return Ok(Capture {
                    details: json!({"strategy": strategy.name(), "source": "embedded-cover",
                        "native_scale": native_scale, "width": screenshot.image.width(),
                        "height": screenshot.image.height(), "raw_width": screenshot.width,
                        "raw_height": screenshot.height, "attempts": 1,
                        "cover_probe_ms": cover_probe_ms,
                        "stop_ms": stop_start.elapsed().as_secs_f64() * 1000.0,
                        "total_capture_ms": start.elapsed().as_secs_f64() * 1000.0}),
                    image: screenshot.image,
                });
            }
            // Restore the original video path after an absent/unreadable cover.
            // This also restores per-file lavf options before normal frame selection.
            self.stop(deadline)?;
        }
        self.load(path, work_deadline)?;
        let load_ms = start.elapsed().as_secs_f64() * 1000.0;
        let duration = selection::duration(self.property("duration").ok().and_then(|v| v.as_f64()));
        let positions = match strategy {
            Strategy::Adaptive => selection::candidates(duration, selection::seed(path)).to_vec(),
            Strategy::Explicit(at) => vec![at],
            Strategy::Legacy => vec![duration.map_or(0.0, |d| (d * 0.1).min(30.0))],
        };
        if let (Strategy::Explicit(at), Some(duration)) = (strategy, duration) {
            ensure!(
                at < duration,
                "Requested timestamp must be before the end of the video"
            );
        }
        let adaptive = matches!(strategy, Strategy::Adaptive);
        let mut best: Option<(usize, Candidate)> = None;
        let mut reports = Vec::new();
        let mut retry_failed = false;
        let mut seek_ms = 0.0;
        let mut capture_ms = 0.0;
        for (index, target) in positions.into_iter().enumerate() {
            if index > 0 && Instant::now() + Duration::from_millis(200) >= work_deadline {
                break;
            }
            // Exact retry prevents several targets from resolving to the same
            // dark keyframe in a long GOP. Very short clips use exact immediately.
            let exact = !adaptive || index > 0 || duration.is_some_and(|d| d <= 1.0);
            let attempt =
                self.candidate(target, exact, adaptive, duration, max_side, work_deadline);
            let candidate = match attempt {
                Ok(candidate) => candidate,
                Err(error) if best.is_none() => return Err(error),
                Err(_) => {
                    retry_failed = true;
                    break;
                }
            };
            seek_ms += candidate.seek_ms;
            capture_ms += candidate.capture_ms;
            reports.push(
                json!({"requested_seconds": candidate.requested, "actual_seconds": candidate.actual,
                "exact_seek": candidate.exact, "keyframe_corrected": candidate.corrected,
                "raw_width": candidate.screenshot.width, "raw_height": candidate.screenshot.height,
                "quality": candidate.quality.json()}),
            );
            let suitable = !candidate.quality.unsuitable;
            let replace = suitable
                || best
                    .as_ref()
                    .is_none_or(|(_, previous)| candidate.quality.score > previous.quality.score);
            if replace {
                best = Some((index, candidate));
            }
            if !adaptive || suitable {
                break;
            }
        }
        let (selected, best) = best.context("No decoded thumbnail candidate")?;
        let mut details = json!({
            "strategy": strategy.name(), "source": "video-frame", "native_scale": native_scale,
            "load_ms": load_ms, "seek_ms": seek_ms, "capture_resize_ms": capture_ms,
            "requested_seconds": best.requested, "actual_seconds": best.actual,
            "duration_seconds": duration, "width": best.screenshot.image.width(), "height": best.screenshot.image.height(),
            "raw_width": best.screenshot.width, "raw_height": best.screenshot.height,
            "attempts": reports.len(), "selected_attempt": selected + 1,
            "quality_fallback": adaptive && best.quality.unsuitable,
            "retry_failed": retry_failed, "quality": best.quality.json(), "candidates": reports,
            "codec": self.property("video-codec").ok(), "video_out": self.property("current-vo").ok(),
            "hwdec": self.property("hwdec-current").ok(), "video_params": self.property("video-params").ok(),
            "decoded_params": self.property("video-dec-params").ok(),
        });
        let stop_start = Instant::now();
        self.stop(deadline)?;
        details["stop_ms"] = (stop_start.elapsed().as_secs_f64() * 1000.0).into();
        details["total_capture_ms"] = (start.elapsed().as_secs_f64() * 1000.0).into();
        Ok(Capture {
            image: best.screenshot.image,
            details,
        })
    }

    fn candidate(
        &mut self,
        target: f64,
        exact: bool,
        adaptive: bool,
        duration: Option<f64>,
        max_side: u32,
        deadline: Instant,
    ) -> Result<Candidate> {
        let seek_start = Instant::now();
        self.seek(target, exact, deadline)?;
        let mut actual = self.property("time-pos").ok().and_then(|v| v.as_f64());
        // Approximation must not undo intro avoidance by jumping to frame zero.
        let corrected = adaptive && !exact && selection::outside_middle(actual, duration);
        if corrected {
            self.seek(target, true, deadline)?;
            actual = self.property("time-pos").ok().and_then(|v| v.as_f64());
        }
        let seek_ms = seek_start.elapsed().as_secs_f64() * 1000.0;
        let capture_start = Instant::now();
        let screenshot = self
            .request(
                &["screenshot-raw", "video"],
                deadline,
                Some(max_side),
                false,
            )?
            .context("Screenshot returned no pixels")?;
        let quality = quality::assess(&screenshot.image);
        Ok(Candidate {
            screenshot,
            quality,
            requested: target,
            actual,
            seek_ms,
            capture_ms: capture_start.elapsed().as_secs_f64() * 1000.0,
            exact: exact || corrected,
            corrected,
        })
    }
}
