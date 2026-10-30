use super::*;
use crate::app::disk_analysis_duplicates::DeleteStatus;

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
        if state.duplicates.deletion.is_running() {
            ui.spinner();
            ui.add(egui::Label::new(t!("disk_analysis.dup_delete_working").to_string()).wrap());
        } else {
            let message = match state.duplicates.deletion.status {
                Some(DeleteStatus::Removed) => t!("disk_analysis.dup_delete_done"),
                Some(DeleteStatus::Unsafe) => t!("disk_analysis.dup_delete_unsafe"),
                Some(DeleteStatus::WorkerFailed) => t!("disk_analysis.dup_delete_error"),
                _ => t!("disk_analysis.dup_delete_cancelled"),
            };
            ui.add(egui::Label::new(message.to_string()).wrap());
            if ui
                .button(t!("disk_analysis.dup_delete_close").to_string())
                .clicked()
            {
                state.duplicates.deletion.cancel();
            }
        }
    });
}
