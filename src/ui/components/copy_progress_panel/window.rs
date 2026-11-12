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
    let screen = ctx.viewport_rect();
    let width = 520.0f32.min((screen.width() - 24.0).max(180.0));
    let mut dismissed = Vec::new();
    egui::Window::new(t!("copy_engine.panel_title"))
        .id(egui::Id::new(WINDOW_ID))
        .default_pos(egui::pos2(
            (screen.right() - width - 24.0).max(screen.left()),
            screen.top() + 70.0,
        ))
        .default_width(width)
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
                            if card(ui, operation, panel.metrics.entry(*id).or_default()) {
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
