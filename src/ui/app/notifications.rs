use eframe::egui;
use crate::app::ImageViewerApp;

pub fn render_notifications(app: &mut ImageViewerApp, ctx: &egui::Context) {
    app.notifications.cleanup();

    if !app.notifications.is_empty() {
        let toast_width = 300.0;
        let toast_height = 40.0;
        let padding = 10.0;
        let margin = 20.0;

        let screen = ctx.screen_rect();
        let base_x = screen.max.x - toast_width - margin;

        for (i, notification) in app.notifications.active().iter().enumerate() {
            let base_y = screen.max.y - margin - ((i + 1) as f32 * (toast_height + padding));
            let fade = notification.remaining_fraction();

            let mut bg_color = notification.level.color();
            bg_color = egui::Color32::from_rgba_unmultiplied(
                bg_color.r(),
                bg_color.g(),
                bg_color.b(),
                (fade * 230.0) as u8,
            );

            egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                .fixed_pos(egui::pos2(base_x, base_y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let rect = egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(toast_width, toast_height),
                    );

                    ui.painter().rect_filled(rect, 6.0, bg_color);

                    // Icon
                    ui.painter().text(
                        rect.min + egui::vec2(12.0, 12.0),
                        egui::Align2::LEFT_TOP,
                        notification.level.icon(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE.gamma_multiply(fade),
                    );

                    // Message
                    ui.painter().text(
                        rect.min + egui::vec2(32.0, 12.0),
                        egui::Align2::LEFT_TOP,
                        &notification.message,
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE.gamma_multiply(fade),
                    );
                });
        }
        ctx.request_repaint(); // Keep animating
    }
}
