use crate::app::state::ImageViewerApp;
use crate::ui::theme;
use eframe::egui::{self, Color32, Margin, RichText, Vec2};
use rust_i18n::t;

const MAX_PREVIEW_ITEMS: usize = 5;
const BACKDROP_ALPHA: u8 = 72;
const MODAL_WIDTH: f32 = 480.0;

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

    let screen_rect = ctx.screen_rect();
    let modal_width = MODAL_WIDTH.min(screen_rect.width() - 32.0);

    // ── Backdrop (blocks interaction outside the modal) ────────────────────────
    let mut close_from_backdrop = false;
    egui::Area::new(egui::Id::from("drag_move_confirm_backdrop"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_size(screen_rect.size());
            let backdrop_rect = ui.max_rect();
            let backdrop_resp = ui.interact(
                backdrop_rect,
                ui.id().with("drag_move_confirm_backdrop_interact"),
                egui::Sense::click(),
            );
            ui.painter().rect_filled(
                backdrop_rect,
                0.0,
                Color32::from_black_alpha(BACKDROP_ALPHA),
            );
            if backdrop_resp.clicked() {
                close_from_backdrop = true;
            }
        });

    if close_from_backdrop {
        app.cancel_pending_drag_move();
        return;
    }

    // ESC cancels
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.cancel_pending_drag_move();
        return;
    }

    let mut confirm = false;
    let mut cancel = false;

    // Estimate content height to center the modal vertically
    let content_width = modal_width - 48.0;
    let estimated_height = {
        let mut h = 0.0_f32;
        h += 22.0; // title
        h += 14.0; // space
        h += 18.0; // summary
        h += 12.0; // space
        h += 48.0; // dest block
        h += 12.0; // space
        if !item_names.is_empty() {
            h += 16.0; // "Items:" label
            h += 4.0;  // space
            for _ in &item_names {
                h += 18.0; // each item line
            }
            if remaining_count > 0 {
                h += 18.0;
            }
            h += 8.0; // space
        }
        h += 8.0;  // space before buttons
        h += 34.0; // buttons
        h += 36.0; // inner margins top+bottom
        h
    };

    let modal_x = (screen_rect.width() - modal_width) / 2.0;
    let modal_y = (screen_rect.height() - estimated_height) / 2.0;

    // ── Modal card ─────────────────────────────────────────────────────────────
    egui::Area::new(egui::Id::from("drag_move_confirm_modal"))
        .fixed_pos(egui::pos2(modal_x, modal_y.max(24.0)))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let dark_mode = ui.visuals().dark_mode;
            let bg_color = if dark_mode {
                Color32::from_rgb(50, 50, 50)
            } else {
                Color32::from_rgb(250, 250, 250)
            };

            egui::Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 20,
                    bottom: 16,
                })
                .corner_radius(10.0)
                .fill(bg_color)
                .stroke(egui::Stroke::new(
                    1.0,
                    if dark_mode {
                        Color32::from_gray(70)
                    } else {
                        Color32::from_gray(220)
                    },
                ))
                .shadow(egui::epaint::Shadow {
                    spread: 4,
                    blur: 12,
                    color: Color32::from_black_alpha(25),
                    offset: [0, 3],
                })
                .show(ui, |ui| {
                    ui.set_width(content_width);

                    // Header
                    ui.label(
                        RichText::new(t!("drag_drop.confirm_move_title"))
                            .size(18.0)
                            .strong()
                            .color(theme::text_color(dark_mode)),
                    );

                    ui.add_space(14.0);

                    // Summary
                    let summary = if count == 1 {
                        t!("drag_drop.confirm_move_summary_one").to_string()
                    } else {
                        t!("drag_drop.confirm_move_summary_many", count = count).to_string()
                    };
                    ui.label(
                        RichText::new(summary).size(14.0).color(theme::text_color(dark_mode)),
                    );

                    ui.add_space(12.0);

                    // Destination block
                    let dest_bg = if dark_mode {
                        Color32::from_rgb(40, 40, 40)
                    } else {
                        Color32::from_rgb(240, 240, 240)
                    };
                    egui::Frame::new()
                        .fill(dest_bg)
                        .inner_margin(Margin::same(10))
                        .corner_radius(6.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(t!("drag_drop.confirm_move_destination"))
                                    .size(12.0)
                                    .color(theme::secondary_text_color(dark_mode)),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(dest_folder)
                                    .size(13.0)
                                    .monospace()
                                    .color(theme::text_color(dark_mode)),
                            );
                        });

                    ui.add_space(12.0);

                    // Items list
                    if !item_names.is_empty() {
                        ui.label(
                            RichText::new(t!("drag_drop.confirm_move_items"))
                                .size(12.0)
                                .color(theme::secondary_text_color(dark_mode)),
                        );
                        ui.add_space(4.0);
                        for name in &item_names {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("•").size(12.0).color(theme::COLOR_ACCENT));
                                ui.label(RichText::new(name).size(12.0).color(theme::text_color(dark_mode)));
                            });
                        }
                        if remaining_count > 0 {
                            ui.label(
                                RichText::new(
                                    t!("drag_drop.confirm_move_more", count = remaining_count).to_string(),
                                )
                                .size(12.0)
                                .color(theme::secondary_text_color(dark_mode))
                                .italics(),
                            );
                        }
                        ui.add_space(8.0);
                    }

                    ui.add_space(8.0);

                    // Buttons (right-aligned, close to bottom)
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized(
                                Vec2::new(90.0, 34.0),
                                egui::Button::new(
                                    RichText::new(t!("drag_drop.confirm_move_confirm"))
                                        .size(14.0)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(theme::COLOR_ACCENT),
                            )
                            .clicked()
                        {
                            confirm = true;
                        }

                        ui.add_space(12.0);

                        if ui
                            .add_sized(
                                Vec2::new(90.0, 34.0),
                                egui::Button::new(
                                    RichText::new(t!("drag_drop.confirm_move_cancel"))
                                        .size(14.0)
                                        .color(theme::secondary_text_color(dark_mode)),
                                )
                            .fill(Color32::TRANSPARENT)
                            .stroke(egui::Stroke::new(1.0, theme::secondary_text_color(dark_mode))),
                            )
                            .clicked()
                        {
                            cancel = true;
                        }
                    });
                });
        });

    if confirm {
        app.confirm_pending_drag_move();
    } else if cancel {
        app.cancel_pending_drag_move();
    }
}
