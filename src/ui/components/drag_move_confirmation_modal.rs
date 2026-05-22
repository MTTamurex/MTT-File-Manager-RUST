use crate::app::state::ImageViewerApp;
use eframe::egui::{self, RichText};
use rust_i18n::t;

const MAX_PREVIEW_ITEMS: usize = 5;

pub fn render_drag_move_confirmation_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some((count, dest_folder, item_names, remaining_count)) =
        app.pending_drag_move_confirmation.as_ref().map(|pending| {
            let item_names: Vec<String> = pending
                .paths
                .iter()
                .take(MAX_PREVIEW_ITEMS)
                .map(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string_lossy().to_string())
                })
                .collect();
            (
                pending.paths.len(),
                pending.dest_folder.to_string_lossy().to_string(),
                item_names,
                pending.paths.len().saturating_sub(MAX_PREVIEW_ITEMS),
            )
        })
    else {
        return;
    };

    let mut open = true;
    let mut confirm = false;
    let mut cancel = false;

    egui::Window::new(t!("drag_drop.confirm_move_title"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            let summary = if count == 1 {
                t!("drag_drop.confirm_move_summary_one").to_string()
            } else {
                t!("drag_drop.confirm_move_summary_many", count = count).to_string()
            };
            ui.label(RichText::new(summary).strong());

            ui.add_space(8.0);
            ui.label(t!("drag_drop.confirm_move_destination"));
            ui.monospace(dest_folder);

            if !item_names.is_empty() {
                ui.add_space(10.0);
                ui.label(t!("drag_drop.confirm_move_items"));
                for name in &item_names {
                    ui.label(format!("- {name}"));
                }
                if remaining_count > 0 {
                    ui.label(
                        t!("drag_drop.confirm_move_more", count = remaining_count).to_string(),
                    );
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui.button(t!("drag_drop.confirm_move_cancel")).clicked() {
                    cancel = true;
                }

                if ui.button(t!("drag_drop.confirm_move_confirm")).clicked() {
                    confirm = true;
                }
            });
        });

    if confirm {
        app.confirm_pending_drag_move();
    } else if cancel || !open {
        app.cancel_pending_drag_move();
    }
}
