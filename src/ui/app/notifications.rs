use crate::app::ImageViewerApp;
use eframe::egui;
use std::time::Duration;

pub fn render_notifications(app: &mut ImageViewerApp, ctx: &egui::Context) {
    app.notifications.cleanup();

    let toast_width = 360.0;
    let toast_min_height: f32 = 52.0;
    let padding = 8.0;
    let margin = 20.0;
    let inner_pad = 14.0;
    let icon_size = 18.0;
    let text_left = inner_pad + icon_size + 10.0;
    let max_text_width = toast_width - text_left - inner_pad;

    let screen = ctx.screen_rect();
    let base_x = screen.max.x - toast_width - margin;

    let mut y_offset = margin;
    let mut needs_repaint = false;

    // === Extraction progress toast (persistent, above regular toasts) ===
    let extraction_state = app
        .file_operation_state
        .extraction_progress
        .lock()
        .ok()
        .and_then(|g| g.clone());

    if let Some(progress) = extraction_state {
        needs_repaint = true;
        let progress_toast_height = 62.0;
        let toast_y = screen.max.y - y_offset - progress_toast_height;
        y_offset += progress_toast_height + padding;

        let bg = egui::Color32::from_rgb(30, 58, 95);
        let accent = egui::Color32::from_rgb(100, 160, 255);
        let border = egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 180);

        egui::Area::new(egui::Id::new("extraction_progress_toast"))
            .fixed_pos(egui::pos2(base_x, toast_y))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(toast_width, progress_toast_height),
                );

                ui.painter().rect_filled(rect, 8.0, bg);
                ui.painter().rect_stroke(
                    rect,
                    8.0,
                    egui::Stroke::new(1.0, border),
                    egui::StrokeKind::Inside,
                );

                // Accent bar
                let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.5, progress_toast_height));
                ui.painter().rect_filled(bar, 0.0, accent);

                // Icon (spinning-style: use a simple arrow)
                ui.painter().text(
                    rect.min + egui::vec2(inner_pad, 10.0),
                    egui::Align2::LEFT_TOP,
                    "📦",
                    egui::FontId::proportional(icon_size),
                    accent,
                );

                // Title + count (truncate archive name to fit toast width)
                let max_name_chars = 30;
                let archive_display = if progress.archive_name.len() > max_name_chars {
                    format!("{}…", &progress.archive_name[..max_name_chars])
                } else {
                    progress.archive_name.clone()
                };
                let title = format!(
                    "{} ({}/{})",
                    archive_display, progress.extracted, progress.total,
                );
                let title_galley = ui.painter().layout(
                    title,
                    egui::FontId::proportional(13.0),
                    egui::Color32::from_rgb(230, 230, 230),
                    max_text_width,
                );
                ui.painter().galley(
                    rect.min + egui::vec2(text_left, 10.0),
                    title_galley,
                    egui::Color32::TRANSPARENT,
                );

                // Current file name (truncated to fit)
                if !progress.current_file.is_empty() {
                    let display_name = if progress.current_file.len() > 35 {
                        format!("…{}", &progress.current_file[progress.current_file.len() - 34..])
                    } else {
                        progress.current_file.clone()
                    };
                    let file_galley = ui.painter().layout(
                        display_name,
                        egui::FontId::proportional(11.0),
                        egui::Color32::from_rgb(160, 170, 190),
                        max_text_width,
                    );
                    ui.painter().galley(
                        rect.min + egui::vec2(text_left, 27.0),
                        file_galley,
                        egui::Color32::TRANSPARENT,
                    );
                }

                // Progress bar
                let bar_y = rect.min.y + progress_toast_height - 12.0;
                let bar_left = rect.min.x + text_left;
                let bar_width = toast_width - text_left - inner_pad;
                let bar_height = 4.0;
                let bar_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_left, bar_y),
                    egui::vec2(bar_width, bar_height),
                );
                ui.painter()
                    .rect_filled(bar_rect, 2.0, egui::Color32::from_rgb(40, 50, 70));

                let fraction = if progress.total > 0 {
                    (progress.extracted as f32 / progress.total as f32).min(1.0)
                } else {
                    0.0
                };
                let fill_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_left, bar_y),
                    egui::vec2(bar_width * fraction, bar_height),
                );
                ui.painter().rect_filled(fill_rect, 2.0, accent);
            });
    }

    // === Regular notification toasts ===
    if !app.notifications.is_empty() {
        needs_repaint = true;

        for (i, notification) in app.notifications.active().iter().enumerate() {
            let fade = notification.remaining_fraction();
            // Smooth fade: stay fully opaque for most of the duration, fade in last 20%
            let alpha = if fade < 0.2 { fade / 0.2 } else { 1.0 };

            // Measure text height to support multi-line
            let galley = ctx.fonts(|f| {
                f.layout(
                    notification.message.clone(),
                    egui::FontId::proportional(13.5),
                    egui::Color32::WHITE,
                    max_text_width,
                )
            });
            let text_height = galley.size().y;
            let toast_height = toast_min_height.max(text_height + inner_pad * 2.0);

            let toast_y = screen.max.y - y_offset - toast_height;
            y_offset += toast_height + padding;

            let bg_color = notification.level.color();
            let accent = notification.level.accent_color();
            let bg = egui::Color32::from_rgba_unmultiplied(
                bg_color.r(), bg_color.g(), bg_color.b(), (alpha * 240.0) as u8,
            );
            let border = egui::Color32::from_rgba_unmultiplied(
                accent.r(), accent.g(), accent.b(), (alpha * 180.0) as u8,
            );

            egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                .fixed_pos(egui::pos2(base_x, toast_y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let rect = egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(toast_width, toast_height),
                    );

                    // Background with border
                    ui.painter().rect_filled(rect, 8.0, bg);
                    ui.painter().rect_stroke(
                        rect,
                        8.0,
                        egui::Stroke::new(1.0, border),
                        egui::StrokeKind::Inside,
                    );

                    // Thin accent bar on the left
                    let bar = egui::Rect::from_min_size(
                        rect.min,
                        egui::vec2(3.5, toast_height),
                    );
                    ui.painter().rect_filled(
                        bar,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(
                            accent.r(), accent.g(), accent.b(), (alpha * 220.0) as u8,
                        ),
                    );

                    // Icon
                    let icon_color = egui::Color32::from_rgba_unmultiplied(
                        accent.r(), accent.g(), accent.b(), (alpha * 255.0) as u8,
                    );
                    ui.painter().text(
                        rect.min + egui::vec2(inner_pad, (toast_height - icon_size) / 2.0),
                        egui::Align2::LEFT_TOP,
                        notification.level.icon(),
                        egui::FontId::proportional(icon_size),
                        icon_color,
                    );

                    // Message text (wrapped)
                    let text_color = egui::Color32::from_rgba_unmultiplied(
                        230, 230, 230, (alpha * 255.0) as u8,
                    );
                    let text_galley = ui.painter().layout(
                        notification.message.clone(),
                        egui::FontId::proportional(13.5),
                        text_color,
                        max_text_width,
                    );
                    let text_y = (toast_height - text_galley.size().y) / 2.0;
                    ui.painter().galley(
                        rect.min + egui::vec2(text_left, text_y),
                        text_galley,
                        egui::Color32::TRANSPARENT,
                    );
                });
        }
    }

    if needs_repaint {
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
