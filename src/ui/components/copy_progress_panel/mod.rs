//! Non-modal copy/deletion dashboard, inheriting the application's theme.
mod active_files;
mod deletion;
mod graph;
pub(crate) mod indicator;
mod summary;
mod window;

use crate::app::copy_panel_state::TransferMetrics;
use crate::app::ImageViewerApp;
use crate::infrastructure::copy_engine::{CopyOperation, OutcomeKind};
use crate::ui::theme;
use eframe::egui::{self, RichText};
use mtt_copy_engine::PauseState;
use rust_i18n::t;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub fn render(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let _stage =
        crate::infrastructure::diagnostic_logger::navigation_trace::Stage::new("copy_dashboard");
    let jobs = app.file_operation_state.copy_runtime.jobs();
    app.file_operation_state
        .copy_panel
        .metrics
        .retain(|id, _| jobs.iter().any(|(job, _)| job == id));
    if jobs.is_empty() {
        app.file_operation_state.copy_panel.restore();
        return;
    }
    // Keep counters and graph sampling current even while the panel is hidden.
    let mut reveal_confirmation = false;
    for (id, operation) in &jobs {
        let metrics = app
            .file_operation_state
            .copy_panel
            .metrics
            .entry(*id)
            .or_default();
        sample(operation, metrics);
        if operation.outcome().is_none() && !operation.progress.is_cancelled() {
            if let Some(request) = operation
                .deletion
                .as_ref()
                .and_then(|deletion| deletion.confirmation.pending())
            {
                if metrics.confirmation_seen != Some(request.id) {
                    metrics.confirmation_seen = Some(request.id);
                    reveal_confirmation = true;
                }
            }
        }
    }
    if jobs.iter().any(|(_, job)| job.outcome().is_none()) {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    if reveal_confirmation {
        app.file_operation_state.copy_panel.restore();
    }
    if app.file_operation_state.copy_panel.minimized {
        return;
    }
    let dismissed = window::show(ctx, &jobs, &mut app.file_operation_state.copy_panel);
    for id in dismissed {
        app.file_operation_state.copy_runtime.dismiss(id);
        ctx.request_repaint();
    }
}

fn sample(operation: &CopyOperation, metrics: &mut TransferMetrics) {
    if operation.deletion.is_some() {
        return;
    }
    let progress = &operation.progress;
    metrics.sample(
        progress.bytes_copied.load(Ordering::Relaxed),
        progress.active_elapsed(),
        operation.outcome().is_none()
            && progress.copying.load(Ordering::Relaxed)
            && !progress.is_cancelled()
            && progress.pause_state() == PauseState::Running,
    );
}

fn card(ui: &mut egui::Ui, operation: &CopyOperation, metrics: &mut TransferMetrics) -> bool {
    let paths = if let Some(deletion) = &operation.deletion {
        deletion.current()
    } else {
        operation.progress.active_files()
    };
    let paths = metrics.names.snapshot(paths, std::time::Instant::now());
    if operation.deletion.is_some() {
        deletion::card(ui, operation, &paths)
    } else {
        card_with_paths(ui, operation, metrics, &paths)
    }
}

fn card_with_paths(
    ui: &mut egui::Ui,
    operation: &CopyOperation,
    metrics: &mut TransferMetrics,
    active_paths: &[std::path::PathBuf],
) -> bool {
    let progress = &operation.progress;
    let outcome = operation.outcome();
    let active = outcome.is_none();
    let copying = progress.copying.load(Ordering::Relaxed);
    let cancelled = progress.is_cancelled();
    let pause = progress.pause_state();
    let bytes = progress.bytes_copied.load(Ordering::Relaxed);
    let total_bytes = progress.total_bytes.load(Ordering::Relaxed);
    let files = progress.files_copied.load(Ordering::Relaxed);
    let total_files = progress.total_files.load(Ordering::Relaxed);
    let mut dismiss = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width((ui.available_width() - 2.0).max(0.0));
        let status = match outcome.as_ref().map(|result| result.kind) {
            Some(OutcomeKind::Completed) => t!("copy_engine.completed"),
            Some(OutcomeKind::Cancelled) => t!("copy_engine.cancelled_title"),
            Some(OutcomeKind::Failed) => t!("copy_engine.failed_title"),
            None if cancelled => t!("copy_engine.cancelling"),
            None if pause == PauseState::Paused => t!("copy_engine.paused"),
            None if pause == PauseState::Pausing => t!("copy_engine.pausing"),
            None if copying => t!("copy_engine.copying"),
            None => t!("copy_engine.preparing"),
        };
        let status_color = if outcome
            .as_ref()
            .is_some_and(|result| result.kind == OutcomeKind::Failed)
        {
            ui.visuals().error_fg_color
        } else {
            theme::accent_color(ui.ctx())
        };
        ui.label(RichText::new(status).strong().color(status_color));
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
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        });
        path_row(
            ui,
            &t!("copy_engine.destination"),
            &operation.destination.display().to_string(),
            || operation.destination.display().to_string(),
        );
        if active {
            let workers = progress
                .telemetry
                .policy()
                .map_or(usize::MAX, |policy| policy.workers);
            active_files::render_paths(ui, active_paths, workers);
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
            let eta = if active && copying && !cancelled && pause == PauseState::Running {
                metrics.remaining(bytes, total_bytes).map(format_duration)
            } else {
                None
            };
            stat(
                &mut columns[1],
                &t!("copy_engine.remaining"),
                eta.unwrap_or_else(|| "—".into()),
            );
        });
        ui.add_space(8.0);
        graph::render(ui, metrics);
        let completed = outcome
            .as_ref()
            .is_some_and(|result| result.kind == OutcomeKind::Completed);
        let fraction = fraction(bytes, total_bytes, files, total_files, completed);
        ui.add_space(8.0);
        ui.add(
            egui::ProgressBar::new(fraction)
                .fill(theme::accent_color(ui.ctx()))
                .show_percentage(),
        );
        ui.add(
            egui::Label::new(format!(
                "{:.1} / {:.1} MiB",
                bytes as f64 / 1_048_576.0,
                total_bytes as f64 / 1_048_576.0
            ))
            .truncate(),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if active {
                let label = if pause == PauseState::Running {
                    t!("copy_engine.pause")
                } else {
                    t!("copy_engine.resume")
                };
                if ui
                    .add_enabled(!cancelled, egui::Button::new(label))
                    .clicked()
                {
                    if pause == PauseState::Running {
                        progress.pause();
                    } else {
                        progress.resume();
                    }
                }
                if ui
                    .add_enabled(!cancelled, egui::Button::new(t!("copy_engine.cancel")))
                    .clicked()
                {
                    progress.cancel();
                }
            } else if ui.button(t!("copy_engine.dismiss")).clicked() {
                dismiss = true;
            }
        });
        if let Some(message) = outcome.as_ref().and_then(|result| result.message.as_ref()) {
            ui.add_space(6.0);
            ui.label(message);
        }
    });
    dismiss
}

fn path_row(ui: &mut egui::Ui, label: &str, value: &str, tooltip: impl FnOnce() -> String) {
    ui.horizontal(|ui| {
        ui.weak(label);
        let response = ui.add(egui::Label::new(value).truncate());
        response.on_hover_ui(|ui| {
            ui.label(tooltip());
        });
    });
}

fn stat(ui: &mut egui::Ui, label: &str, value: String) {
    ui.add(egui::Label::new(RichText::new(label).weak()).truncate());
    ui.add(egui::Label::new(RichText::new(&value).strong()).truncate())
        .on_hover_text(value);
}

fn fraction(bytes: u64, total: u64, files: usize, total_files: usize, completed: bool) -> f32 {
    if completed {
        1.0
    } else if total > 0 {
        (bytes as f64 / total as f64).clamp(0.0, 1.0) as f32
    } else if total_files > 0 {
        (files as f64 / total_files as f64).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{} h {:02} min {:02} s",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn elapsed_duration_formats_seconds_minutes_and_hours() {
        assert_eq!(format_duration(Duration::ZERO), "0 min 00 s");
        assert_eq!(format_duration(Duration::from_secs(65)), "1 min 05 s");
        assert_eq!(
            format_duration(Duration::from_secs(3665)),
            "1 h 01 min 05 s"
        );
    }

    #[test]
    fn progress_handles_empty_files_rollback_and_completed_empty_trees() {
        assert_eq!(fraction(0, 0, 0, 0, false), 0.0);
        assert_eq!(fraction(0, 0, 1, 2, false), 0.5);
        assert_eq!(fraction(0, 0, 0, 0, true), 1.0);
        assert_eq!(fraction(120, 100, 1, 1, false), 1.0);
    }

    #[test]
    fn cards_render_in_application_themes_without_changing_operation_state() {
        use crate::app::navigation_state::ThemeMode;
        use crate::infrastructure::copy_engine::{CopyRuntime, Outcome};
        use std::path::PathBuf;
        use std::sync::Arc;
        let runtime = Arc::new(CopyRuntime::default());
        let job = runtime.start(
            vec![PathBuf::from("source/file.txt")],
            PathBuf::from("target"),
        );
        let ctx = egui::Context::default();
        let mut metrics = TransferMetrics::default();
        for mode in [
            ThemeMode::Light,
            ThemeMode::Dark,
            ThemeMode::DarkDeep,
            ThemeMode::GitHubDark,
        ] {
            theme::apply_theme_visuals(&ctx, mode);
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(!card(ui, &job.operation, &mut metrics));
            });
            assert!(!output.shapes.is_empty());
            assert!(!job.operation.progress.is_cancelled());
        }
        job.finish(Outcome {
            kind: OutcomeKind::Failed,
            message: Some("Copy error".into()),
        });
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(!card(ui, &job.operation, &mut metrics));
        });
        assert_eq!(job.operation.outcome().unwrap().kind, OutcomeKind::Failed);
    }
}
