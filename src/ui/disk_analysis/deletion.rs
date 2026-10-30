use super::*;
use crate::app::disk_analysis_duplicates::{
    DeleteFailure, DeleteProgress, DeleteStage, DeleteStatus,
};
use std::time::Duration;

#[cfg(test)]
mod tests;

pub(super) fn render(state: &mut DiskAnalysisState, ctx: &egui::Context) {
    if !state.duplicates.deletion.is_open() {
        return;
    }
    egui::Modal::new(egui::Id::new("duplicate_delete_status")).show(ctx, |ui| {
        ui.set_width(600.0_f32.min((ctx.viewport_rect().width() - 48.0).max(240.0)));
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 10.0);
        ui.heading(t!("disk_analysis.dup_delete_file").to_string());
        let deletion = &state.duplicates.deletion;
        let progress = deletion.progress();
        let running = deletion.is_running();
        egui::ScrollArea::vertical()
            .id_salt("duplicate_delete_steps")
            .max_height((ctx.viewport_rect().height() - 160.0).max(100.0))
            .show(ui, |ui| {
                render_progress(ui, progress, running, deletion.status);
                if running {
                    // Also refresh while blocked in a file read or a native dialog.
                    // No worker event queue, extra reads, or continuous progress messages.
                    ctx.request_repaint_after(Duration::from_millis(100));
                    let key = if progress.stage == DeleteStage::Windows {
                        "disk_analysis.dup_delete_waiting_windows"
                    } else {
                        "disk_analysis.dup_delete_working"
                    };
                    ui.add(egui::Label::new(t!(key).to_string()).wrap());
                } else {
                    let message = match deletion.status {
                        Some(DeleteStatus::Removed) => t!("disk_analysis.dup_delete_done"),
                        Some(DeleteStatus::Unsafe) => t!("disk_analysis.dup_delete_unsafe"),
                        Some(DeleteStatus::WorkerFailed) => t!("disk_analysis.dup_delete_error"),
                        _ => t!("disk_analysis.dup_delete_cancelled"),
                    };
                    ui.add(egui::Label::new(message.to_string()).wrap());
                }
            });
        if !running
            && ui
                .button(t!("disk_analysis.dup_delete_close").to_string())
                .clicked()
        {
            state.duplicates.deletion.cancel();
        }
    });
}

fn stage_label(stage: DeleteStage) -> String {
    t!(match stage {
        DeleteStage::Pair => "disk_analysis.dup_delete_step_pair",
        DeleteStage::Metadata => "disk_analysis.dup_delete_step_metadata",
        DeleteStage::TargetContent => "disk_analysis.dup_delete_step_target",
        DeleteStage::SurvivorContent => "disk_analysis.dup_delete_step_survivor",
        DeleteStage::Revalidate => "disk_analysis.dup_delete_step_revalidate",
        DeleteStage::Windows => "disk_analysis.dup_delete_step_windows",
    })
    .to_string()
}

fn render_progress(
    ui: &mut egui::Ui,
    progress: DeleteProgress,
    running: bool,
    status: Option<DeleteStatus>,
) {
    for stage in DeleteStage::ALL {
        let key = if status == Some(DeleteStatus::Removed)
            || (stage as usize) < progress.stage as usize
        {
            "disk_analysis.dup_delete_step_done"
        } else if stage != progress.stage {
            "disk_analysis.dup_delete_step_waiting"
        } else if running && progress.failure.is_none() {
            "disk_analysis.dup_delete_step_running"
        } else {
            "disk_analysis.dup_delete_step_stopped"
        };
        ui.horizontal_wrapped(|ui| {
            if stage == progress.stage && running && progress.failure.is_none() {
                ui.spinner();
            }
            ui.label(format!("{} — {}", stage_label(stage), t!(key)));
        });
        if stage == progress.stage {
            if let Some(fraction) = progress.fraction() {
                ui.add(egui::ProgressBar::new(fraction).show_percentage());
                ui.label(
                    t!(
                        "disk_analysis.dup_delete_read_bytes",
                        read = progress.bytes_read.to_string(),
                        total = progress.total_bytes.to_string()
                    )
                    .to_string(),
                );
            }
        }
    }
    if let Some(failure) = progress.failure {
        let key = match failure {
            DeleteFailure::InvalidPair => "disk_analysis.dup_delete_reason_pair",
            DeleteFailure::TargetUnavailableOrChanged => "disk_analysis.dup_delete_reason_target",
            DeleteFailure::SurvivorUnavailableOrChanged => {
                "disk_analysis.dup_delete_reason_survivor"
            }
            DeleteFailure::TargetReadFailed => "disk_analysis.dup_delete_reason_target_read",
            DeleteFailure::SurvivorReadFailed => "disk_analysis.dup_delete_reason_survivor_read",
            DeleteFailure::ContentMismatch => "disk_analysis.dup_delete_reason_content",
            DeleteFailure::PathsChanged => "disk_analysis.dup_delete_reason_paths",
        };
        ui.add(egui::Label::new(t!(key).to_string()).wrap());
    }
}
