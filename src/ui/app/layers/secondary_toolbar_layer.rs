use crate::app::ImageViewerApp;
use eframe::egui;

mod actions;
mod sort_controls;
mod view_zoom_controls;

pub(crate) fn render_secondary_toolbar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let separator_color = if ctx.style().visuals.dark_mode {
        egui::Color32::from_rgb(80, 80, 80)
    } else {
        egui::Color32::from_rgb(210, 210, 210)
    };

    egui::TopBottomPanel::top("secondary_nav_bar")
        .show_separator_line(false)
        .exact_height(46.0)
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::WHITE
            },
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 7,
                bottom: 7,
            },
            ..Default::default()
        })
        .show(ctx, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.bottom(),
                egui::Stroke::new(1.0, separator_color),
            );

            ui.horizontal(|ui| {
                let content_width =
                    6.0 * 28.0 + 30.0 + 110.0 + 2.0 * 28.0 + 80.0 + 80.0 + 3.0 * 8.0 + 16.0 * 12.0;
                let available = ui.available_width();
                let left_pad = ((available - content_width) / 2.0).max(0.0);
                ui.add_space(left_pad);

                ui.spacing_mut().item_spacing = egui::vec2(12.0, 0.0);

                let action = actions::render_action_buttons(ui, app);
                ui.separator();
                sort_controls::render_sort_controls(ui, app);
                ui.separator();
                view_zoom_controls::render_view_and_zoom_controls(ui, app);
                actions::execute_action(action, app);
            });
        });
}
