//! Deletion cards share the copy dashboard's frame, typography and controls.
use super::{active_files, format_duration, path_row, stat};
use crate::infrastructure::copy_engine::{CopyOperation, OutcomeKind};
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
mod confirmation;

pub(super) fn card(ui: &mut egui::Ui, operation: &CopyOperation, paths: &[PathBuf]) {
    let progress = operation.deletion.as_ref().unwrap();
    let outcome = operation.outcome();
    let active = outcome.is_none();
    let cancelled = operation.progress.is_cancelled();
    let pending = if active && !cancelled {
        progress.confirmation.pending()
    } else {
        None
    };
    let removed = progress.removed.load(Ordering::Relaxed);
    let failures = progress.failures.load(Ordering::Relaxed);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width((ui.available_width() - 2.0).max(0.0));
        // Confirmation is its own phase: progress statistics and source/mode
        // summaries only become useful after the user has answered.
        if let Some(request) = &pending {
            confirmation::render(ui, operation, request);
            return;
        }
        let status = match outcome.as_ref().map(|outcome| outcome.kind) {
            Some(OutcomeKind::Completed) => t!("delete_engine.completed"),
            Some(OutcomeKind::Cancelled) => t!("delete_engine.cancelled_title"),
            Some(OutcomeKind::Failed) => t!("delete_engine.failed_title"),
            None if cancelled => t!("copy_engine.cancelling"),
            None if progress.permanently || progress.permanent_phase.load(Ordering::Relaxed) => {
                t!("delete_engine.deleting")
            }
            None => t!("delete_engine.recycling"),
        };
        let color = if failures > 0
            || outcome
                .as_ref()
                .is_some_and(|result| result.kind == OutcomeKind::Failed)
        {
            ui.visuals().error_fg_color
        } else {
            theme::accent_color(ui.ctx())
        };
        ui.add(egui::Label::new(RichText::new(status.clone()).strong().color(color)).truncate());
        ui.add_space(6.0);
        let source = if operation.sources.len() == 1 {
            operation.sources[0].display().to_string()
        } else {
            t!("copy_engine.source_count", count = operation.sources.len()).to_string()
        };
        path_row(ui, &t!("copy_engine.source"), &source, || {
            operation
                .sources
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        });
        let mode = if progress.permanently {
            t!("delete_engine.permanent")
        } else if progress.used_permanent_fallback.load(Ordering::Relaxed) {
            t!("delete_engine.recycle_with_fallback")
        } else {
            t!("delete_engine.recycle_bin")
        };
        path_row(ui, &t!("delete_engine.mode"), &mode, || mode.to_string());
        if active {
            active_files::render_paths(ui, paths, 1);
        }
        ui.add_space(10.0);
        ui.columns(2, |columns| {
            stat(
                &mut columns[0],
                &if active {
                    t!("copy_engine.elapsed")
                } else {
                    t!("copy_engine.total_time")
                },
                format_duration(operation.elapsed()),
            );
            stat(
                &mut columns[1],
                &t!("delete_engine.removed"),
                removed.to_string(),
            );
        });
        ui.add_space(8.0);
        if active {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak(t!("delete_engine.unknown_total"));
            });
        } else if !outcome
            .as_ref()
            .is_some_and(|result| result.kind == OutcomeKind::Cancelled)
        {
            let complete = outcome
                .as_ref()
                .is_some_and(|result| result.kind == OutcomeKind::Completed);
            ui.add(
                egui::ProgressBar::new(if complete { 1.0 } else { 0.0 })
                    .fill(theme::accent_color(ui.ctx()))
                    .text(status),
            );
        }
        if failures > 0 {
            ui.label(t!("delete_engine.failed", count = failures));
        }
        ui.add_space(6.0);
        if active {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!cancelled, egui::Button::new(t!("copy_engine.cancel")))
                        .clicked()
                    {
                        operation.progress.cancel();
                    }
                });
            });
        }
        if let Some(message) = outcome.as_ref().and_then(|result| result.message.as_ref()) {
            ui.add_space(6.0);
            ui.label(message);
        }
    });
}

#[cfg(test)]
mod tests;
