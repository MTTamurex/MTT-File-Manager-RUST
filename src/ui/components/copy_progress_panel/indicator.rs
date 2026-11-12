//! Status-bar entry for a hidden copy panel. Reading it never changes a job.
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::{CopyRuntime, OutcomeKind};
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::sync::atomic::Ordering;

pub(crate) struct CopyIndicator {
    deletion: bool,
    pub count: usize,
    pub active: usize,
    pub finished: usize,
    pub failed: usize,
    pub fraction: Option<f32>,
}

impl CopyIndicator {
    pub fn from_runtime(runtime: &CopyRuntime) -> Option<Self> {
        let jobs = runtime.jobs();
        if jobs.is_empty() {
            return None;
        }
        let mut result = Self {
            deletion: jobs.iter().any(|(_, job)| job.deletion.is_some()),
            count: jobs.len(),
            active: 0,
            finished: 0,
            failed: 0,
            fraction: None,
        };
        let (mut bytes, mut total, mut files, mut total_files) = (0u64, 0u64, 0usize, 0usize);
        let mut preparing = false;
        for (_, job) in jobs {
            if let Some(outcome) = job.outcome() {
                result.finished += 1;
                result.failed += usize::from(outcome.kind == OutcomeKind::Failed);
                continue;
            }
            result.active += 1;
            let progress = &job.progress;
            if job.deletion.is_some() {
                preparing = true;
                continue;
            }
            preparing |= !progress.copying.load(Ordering::Relaxed);
            bytes = bytes.saturating_add(progress.bytes_copied.load(Ordering::Relaxed));
            total = total.saturating_add(progress.total_bytes.load(Ordering::Relaxed));
            files = files.saturating_add(progress.files_copied.load(Ordering::Relaxed));
            total_files = total_files.saturating_add(progress.total_files.load(Ordering::Relaxed));
        }
        if !preparing && result.active > 0 {
            result.fraction = if total > 0 {
                Some((bytes as f64 / total as f64).clamp(0.0, 1.0) as f32)
            } else if total_files > 0 {
                Some((files as f64 / total_files as f64).clamp(0.0, 1.0) as f32)
            } else {
                None
            };
        }
        Some(result)
    }

    fn label(&self) -> String {
        if self.deletion {
            return t!(
                "delete_engine.indicator",
                count = self.count,
                active = self.active,
                failed = self.failed
            )
            .to_string();
        }
        if self.active > 0 {
            let progress = self
                .fraction
                .map(|fraction| format!("{:.0}%", fraction * 100.0))
                .unwrap_or_else(|| "…".into());
            t!(
                "copy_engine.indicator_active",
                count = self.count,
                progress = progress
            )
            .to_string()
        } else if self.failed > 0 {
            t!("copy_engine.indicator_errors", count = self.count).to_string()
        } else {
            t!("copy_engine.indicator_results", count = self.count).to_string()
        }
    }

    pub fn width(&self, ui: &egui::Ui) -> f32 {
        let galley = ui.painter().layout_no_wrap(
            self.label(),
            egui::TextStyle::Button.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        // Leave room for the existing status-bar controls; long translations
        // are elided, with the complete summary available on hover.
        (galley.size().x + ui.spacing().button_padding.x * 2.0 + 12.0)
            .min(ui.available_width() * 0.4)
    }
}

pub(crate) fn render(
    ui: &mut egui::Ui,
    summary: &CopyIndicator,
    panel: &mut CopyPanelState,
) -> egui::Response {
    let color = if summary.failed > 0 {
        ui.visuals().error_fg_color
    } else {
        theme::accent_color(ui.ctx())
    };
    let response = ui
        .add_sized(
            [ui.available_width(), ui.spacing().interact_size.y],
            egui::Button::new(RichText::new(summary.label()).color(color)).truncate(),
        )
        .on_hover_text(t!(
            if summary.deletion {
                "delete_engine.restore_tooltip"
            } else {
                "copy_engine.restore_tooltip"
            },
            active = summary.active,
            finished = summary.finished,
            failed = summary.failed
        ));
    if response.clicked() {
        panel.restore();
        ui.ctx().request_repaint();
    }
    response
}

/// Separate hit-test/paint areas so the badge never covers status-bar controls.
pub(crate) fn rects(
    full: egui::Rect,
    width: f32,
    row_height: f32,
    below: bool,
) -> (egui::Rect, egui::Rect) {
    if width <= 0.0 {
        return (full, egui::Rect::NOTHING);
    }
    let status = egui::Rect::from_min_max(
        full.min,
        if below {
            egui::pos2(full.right(), full.bottom() - row_height)
        } else {
            egui::pos2(full.right() - width, full.bottom())
        },
    );
    let badge = egui::Rect::from_min_max(
        egui::pos2(
            full.right() - width + 4.0f32.min(width),
            if below { status.bottom() } else { full.top() },
        ),
        full.max,
    );
    (status, badge)
}

#[cfg(test)]
mod tests;
