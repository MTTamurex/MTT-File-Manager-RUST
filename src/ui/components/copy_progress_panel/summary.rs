//! Title summary reads only operation state and already-sampled speeds.
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::{CopyOperation, OutcomeKind};
use mtt_copy_engine::PauseState;
use rust_i18n::t;
use std::sync::{atomic::Ordering, Arc};

pub(super) struct Summary {
    removed: Option<super::deletion_counts::Counts>,
    has_copies: bool,
    fraction: Option<f64>,
    preparing: bool,
    files: usize,
    total_files: usize,
    speed: f64,
}

impl Summary {
    pub(super) fn from_jobs(
        jobs: &[(u64, Arc<CopyOperation>)],
        panel: &CopyPanelState,
    ) -> Option<Self> {
        if jobs.is_empty() {
            return None;
        }
        let snapshots: Vec<_> = jobs
            .iter()
            .map(|(id, job)| (*id, job, job.outcome()))
            .collect();
        let active = snapshots.iter().any(|(_, _, outcome)| outcome.is_none());
        let (mut bytes, mut total_bytes) = (0u128, 0u128);
        let (mut files, mut total_files) = (0usize, 0usize);
        let mut speed = 0.0;
        let mut preparing = false;
        let mut completed = true;
        let mut removed = None::<super::deletion_counts::Counts>;
        let mut has_copies = false;
        for (id, job, outcome) in snapshots {
            // Retained old results must not skew a newly started copy.
            if active && outcome.is_some() {
                continue;
            }
            if let Some(deletion) = &job.deletion {
                removed
                    .get_or_insert_with(Default::default)
                    .include(deletion);
                continue;
            }
            has_copies = true;
            let progress = &job.progress;
            let copying = progress.copying.load(Ordering::Relaxed);
            preparing |= outcome.is_none() && !copying;
            let done = outcome
                .as_ref()
                .is_some_and(|result| result.kind == OutcomeKind::Completed);
            completed &= done;
            let total = progress.total_bytes.load(Ordering::Relaxed);
            let copied = progress.bytes_copied.load(Ordering::Relaxed).min(total);
            total_bytes += u128::from(total);
            bytes += u128::from(if done { total } else { copied });
            let total_count = progress.total_files.load(Ordering::Relaxed);
            total_files = total_files.saturating_add(total_count);
            files = files.saturating_add(if done {
                total_count
            } else {
                progress
                    .files_copied
                    .load(Ordering::Relaxed)
                    .min(total_count)
            });
            if outcome.is_none()
                && copying
                && !progress.is_cancelled()
                && progress.pause_state() == PauseState::Running
            {
                if let Some(metrics) = panel.metrics.get(&id) {
                    if metrics.speed.is_finite() && metrics.speed >= 0.0 {
                        speed += metrics.speed;
                    }
                }
            }
        }
        let fraction = if preparing {
            None
        } else if completed {
            Some(1.0)
        } else if total_bytes > 0 {
            Some(bytes as f64 / total_bytes as f64)
        } else if total_files > 0 {
            Some(files as f64 / total_files as f64)
        } else {
            None
        };
        Some(Self {
            removed,
            has_copies,
            fraction,
            preparing,
            files,
            total_files,
            speed: speed.min(f64::MAX),
        })
    }

    pub(super) fn label(&self) -> String {
        if !self.has_copies {
            return self.removed.as_ref().map_or_else(
                || super::deletion_counts::Counts::default().label(),
                |counts| counts.label(),
            );
        }
        let progress = self
            .fraction
            .map(|fraction| format!("{:.0}%", fraction * 100.0))
            .unwrap_or_else(|| {
                if self.preparing {
                    "…".into()
                } else {
                    "—".into()
                }
            });
        let copy = format!(
            "{progress} · {} · {:.1} MiB/s",
            t!(
                "copy_engine.header_files",
                copied = self.files,
                total = self.total_files
            ),
            self.speed / 1_048_576.0
        );
        if let Some(removed) = &self.removed {
            format!("{copy} · {}", removed.label())
        } else {
            copy
        }
    }
}

#[cfg(test)]
mod tests;
