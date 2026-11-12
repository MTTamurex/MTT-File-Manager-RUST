//! Full copy-window layout, shared by the application and geometry regression tests.
use super::card;
use crate::app::copy_panel_state::CopyPanelState;
use crate::infrastructure::copy_engine::CopyOperation;
use eframe::egui;
use rust_i18n::t;
use std::sync::Arc;

const WINDOW_ID: &str = "mtt_copy_progress_panel";

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
    ) -> bool,
) -> Vec<u64> {
    let screen = ctx.viewport_rect();
    let width = 520.0f32.min((screen.width() - 24.0).max(180.0));
    let mut dismissed = Vec::new();
    let title = super::summary::Summary::from_jobs(jobs, panel)
        .map(|summary| summary.label())
        .unwrap_or_else(|| t!("copy_engine.preparing").to_string());
    egui::Window::new(egui::RichText::new(title).text_style(egui::TextStyle::Button))
        .id(egui::Id::new(WINDOW_ID))
        .default_pos(egui::pos2(
            (screen.right() - width - 24.0).max(screen.left()),
            screen.top() + 70.0,
        ))
        .default_width(width)
        // Let auto-shrinking content fit at large font scales instead of
        // clipping controls inside egui's default 420-point resize height.
        .default_height((screen.height() - 150.0).max(100.0))
        .max_width((screen.width() - 24.0).max(180.0))
        .min_width(width.min(320.0))
        .resizable(true)
        .collapsible(true)
        .show(ctx, |ui| {
            // Bound the alignment container to one row. Using with_layout
            // directly here consumes the resizable window's remaining height.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t!("copy_engine.minimize")).clicked() {
                        panel.minimize();
                        ctx.request_repaint();
                    }
                });
            });
            egui::ScrollArea::vertical()
                .max_height((screen.height() - 150.0).max(100.0))
                .show(ui, |ui| {
                    for (id, operation) in jobs {
                        ui.push_id(id, |ui| {
                            if render_card(ui, operation, panel.metrics.entry(*id).or_default()) {
                                dismissed.push(*id);
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
        });
    dismissed
}

#[cfg(test)]
mod tests;
