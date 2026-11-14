//! Full copy-window layout, shared by the application and geometry regression tests.
use super::card;
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::CopyOperation;
use eframe::egui;
use rust_i18n::t;
use std::sync::Arc;

const WINDOW_ID: &str = "mtt_copy_progress_panel";
pub(super) mod controls;

pub(super) fn show(
    ctx: &egui::Context,
    jobs: &[(u64, Arc<CopyOperation>)],
    panel: &mut CopyPanelState,
) -> Vec<u64> {
    show_with(ctx, jobs, panel, card)
}

fn show_with(
    ctx: &egui::Context,
    jobs: &[(u64, Arc<CopyOperation>)],
    panel: &mut CopyPanelState,
    mut render_card: impl FnMut(
        &mut egui::Ui,
        &CopyOperation,
        &mut crate::app::copy_panel_state::TransferMetrics,
    ),
) -> Vec<u64> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let screen = ctx.viewport_rect();
    let width = 520.0f32.min((screen.width() - 24.0).max(180.0));
    let newest_operation = jobs.iter().map(|(id, _)| *id).max();
    let center_operation = panel.pending_initial_center
        || newest_operation
            .is_some_and(|id| panel.last_centered_operation.is_none_or(|seen| id > seen));
    let saved_area = egui::AreaState::load(ctx, egui::Id::new(WINDOW_ID));
    let saved_pivot = saved_area.map_or(egui::Align2::CENTER_CENTER, |state| state.pivot);
    let mut dismissed = Vec::new();
    let copy_conflict = jobs.iter().any(|(_, job)| {
        job.outcome().is_none()
            && !job.progress.is_cancelled()
            && job.progress.conflicts.pending().is_some()
    });
    let confirming = jobs.iter().any(|(_, job)| {
        job.outcome().is_none()
            && !job.progress.is_cancelled()
            && job
                .deletion
                .as_ref()
                .is_some_and(|deletion| deletion.confirmation.pending().is_some())
    });
    let title = super::summary::Summary::from_jobs(jobs, panel)
        .map(|summary| summary.label())
        .unwrap_or_else(|| t!("copy_engine.preparing").to_string());
    let title = if copy_conflict {
        t!("copy_engine.conflict_title").to_string()
    } else if confirming {
        t!("delete_engine.awaiting_confirmation").to_string()
    } else {
        title
    };
    let mut window =
        egui::Window::new(egui::RichText::new(&title).text_style(egui::TextStyle::Button))
            .id(egui::Id::new(WINDOW_ID))
            .pivot(saved_pivot)
            .default_pos(screen.center())
            .default_width(width)
            // Let auto-shrinking content fit at large font scales instead of
            // clipping controls inside egui's default 420-point resize height.
            .default_height((screen.height() - 150.0).max(100.0))
            .max_width((screen.width() - 24.0).max(180.0))
            .min_width(width.min(320.0))
            .resizable(true)
            .title_bar(false);
    if center_operation {
        panel.last_centered_operation = newest_operation;
        // A fresh area needs one placement after its initial sizing pass too.
        panel.pending_initial_center = saved_area.is_none_or(|state| state.size.is_none());
        // Center the actual, dynamically sized window, not an estimated height.
        // Disable dragging only for this placement frame so egui does not
        // restore the old title-drag position over current_pos().
        window = window
            .pivot(egui::Align2::CENTER_CENTER)
            .current_pos(screen.center())
            // The first layout can still carry an estimated/cached size.
            // Constrain the measured window next frame, not that estimate.
            .constrain(false)
            .movable(false);
        ctx.request_repaint();
    }
    window.show(ctx, |ui| {
        let active = jobs
            .iter()
            .any(|(_, operation)| operation.outcome().is_none());
        let (minimize, close) = controls::header(ui, &title, active);
        if close {
            if active {
                for (_, operation) in jobs {
                    operation.close();
                }
                // Closed jobs are no longer dashboard/indicator entries. Their
                // workers remain alive to finish cancellation in the background.
                panel.restore();
            } else {
                dismissed.extend(jobs.iter().map(|(id, _)| *id));
            }
            ctx.request_repaint();
        } else if minimize {
            panel.minimize();
            ctx.request_repaint();
        }
        egui::ScrollArea::vertical()
            .max_height((screen.height() - 150.0).max(100.0))
            .show(ui, |ui| {
                for (id, operation) in jobs {
                    ui.push_id(id, |ui| {
                        render_card(ui, operation, panel.metrics.entry(*id).or_default());
                    });
                    ui.add_space(8.0);
                }
            });
    });
    dismissed
}

#[cfg(test)]
mod tests;
