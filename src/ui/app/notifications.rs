use crate::app::ImageViewerApp;
use crate::ui::preview_panel::utils::truncate_text_to_fit;
use eframe::egui;
use rust_i18n::t;
use std::time::Duration;

fn truncate_end(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let truncated: String = text.chars().take(max_chars).collect();
    format!("{}…", truncated)
}

fn truncate_tail(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let tail_len = max_chars.saturating_sub(1);
    let suffix: String = text.chars().skip(text.chars().count() - tail_len).collect();
    format!("…{}", suffix)
}

#[allow(clippy::too_many_arguments)]
fn render_progress_toast(
    ctx: &egui::Context,
    id: egui::Id,
    pos: egui::Pos2,
    toast_width: f32,
    toast_height: f32,
    inner_pad: f32,
    icon_size: f32,
    text_left: f32,
    max_text_width: f32,
    bg: egui::Color32,
    accent: egui::Color32,
    icon: &str,
    title: String,
    subtitle: Option<String>,
    progress: Option<(usize, usize)>,
    show_cancel: bool,
) -> bool {
    let mut cancelled = false;
    egui::Area::new(id)
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let rect =
                egui::Rect::from_min_size(ui.cursor().min, egui::vec2(toast_width, toast_height));
            let title_font = egui::FontId::proportional(13.0);
            let subtitle_font = egui::FontId::proportional(11.0);

            ui.painter().rect_filled(rect, 8.0, bg);
            ui.painter().text(
                rect.min + egui::vec2(inner_pad, 10.0),
                egui::Align2::LEFT_TOP,
                icon,
                egui::FontId::proportional(icon_size),
                accent,
            );

            let title = truncate_text_to_fit(&title, max_text_width, &title_font, ui);
            let title_galley = ui.painter().layout_no_wrap(
                title,
                title_font,
                egui::Color32::from_rgb(230, 230, 230),
            );
            ui.painter().galley(
                rect.min + egui::vec2(text_left, 10.0),
                title_galley,
                egui::Color32::TRANSPARENT,
            );

            if let Some(subtitle) = subtitle.filter(|text| !text.is_empty()) {
                let subtitle = truncate_text_to_fit(&subtitle, max_text_width, &subtitle_font, ui);
                let subtitle_galley = ui.painter().layout_no_wrap(
                    subtitle,
                    subtitle_font,
                    egui::Color32::from_rgb(160, 170, 190),
                );
                ui.painter().galley(
                    rect.min + egui::vec2(text_left, 27.0),
                    subtitle_galley,
                    egui::Color32::TRANSPARENT,
                );
            }

            if show_cancel {
                let btn_size = 18.0;
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.max.x - inner_pad - btn_size, rect.min.y + 6.0),
                    egui::vec2(btn_size, btn_size),
                );
                let btn_response = ui
                    .allocate_rect(btn_rect, egui::Sense::click())
                    .on_hover_text(t!("extract.cancel_tooltip").to_string());
                let btn_color = if btn_response.hovered() {
                    egui::Color32::from_rgb(255, 100, 100)
                } else {
                    egui::Color32::from_rgb(180, 180, 180)
                };
                ui.painter().text(
                    btn_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "✕",
                    egui::FontId::proportional(14.0),
                    btn_color,
                );
                if btn_response.clicked() {
                    cancelled = true;
                }
            }

            let bar_y = rect.min.y + toast_height - 12.0;
            let bar_left = rect.min.x + text_left;
            let bar_width = toast_width - text_left - inner_pad;
            let bar_height = 4.0;
            let bar_rect = egui::Rect::from_min_size(
                egui::pos2(bar_left, bar_y),
                egui::vec2(bar_width, bar_height),
            );
            ui.painter()
                .rect_filled(bar_rect, 2.0, egui::Color32::from_rgb(40, 50, 70));

            if let Some((done, total)) = progress.filter(|(_, total)| *total > 0) {
                let fraction = (done as f32 / total as f32).min(1.0);
                let fill_rect = egui::Rect::from_min_size(
                    egui::pos2(bar_left, bar_y),
                    egui::vec2(bar_width * fraction, bar_height),
                );
                ui.painter().rect_filled(fill_rect, 2.0, accent);
            } else {
                let t = ui.ctx().input(|i| i.time) as f32;
                let cycle = (t * 0.8).sin() * 0.5 + 0.5;
                let highlight_width = bar_width * 0.3;
                let highlight_x = bar_left + cycle * (bar_width - highlight_width);
                let fill_rect = egui::Rect::from_min_size(
                    egui::pos2(highlight_x, bar_y),
                    egui::vec2(highlight_width, bar_height),
                );
                ui.painter().rect_filled(fill_rect, 2.0, accent);
            }
        });
    cancelled
}

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

        let archive_display = truncate_end(&progress.archive_name, 30);
        let total_known = progress.total > 0;
        let subtitle = if total_known {
            format!(
                "{} ({}/{})",
                archive_display, progress.extracted, progress.total
            )
        } else if progress.extracted > 0 {
            format!(
                "{} ({} {})",
                archive_display,
                progress.extracted,
                t!("extract.items_extracted")
            )
        } else {
            format!("{} — {}", archive_display, t!("extract.preparing"))
        };

        let cancelled = render_progress_toast(
            ctx,
            egui::Id::new("extraction_progress_toast"),
            egui::pos2(base_x, toast_y),
            toast_width,
            progress_toast_height,
            inner_pad,
            icon_size,
            text_left,
            max_text_width,
            egui::Color32::from_rgb(30, 58, 95),
            egui::Color32::from_rgb(100, 160, 255),
            "📦",
            t!("extract.title").to_string(),
            Some(subtitle),
            if total_known {
                Some((progress.extracted, progress.total))
            } else {
                None
            },
            true,
        );

        if cancelled {
            app.file_operation_state
                .extraction_cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if let Ok(mut guard) = app.file_operation_state.extraction_progress.lock() {
                *guard = None;
            }
        }
    }

    if let Some(progress) = app.file_operation_state.batch_rename_progress.clone() {
        if progress.total > 0 {
            needs_repaint = true;
            let progress_toast_height = 62.0;
            let toast_y = screen.max.y - y_offset - progress_toast_height;
            y_offset += progress_toast_height + padding;

            let completed = progress.completed.min(progress.total);
            let title = format!(
                "{} ({}/{})",
                t!("batch_rename.progress_title"),
                completed,
                progress.total,
            );

            render_progress_toast(
                ctx,
                egui::Id::new("batch_rename_progress_toast"),
                egui::pos2(base_x, toast_y),
                toast_width,
                progress_toast_height,
                inner_pad,
                icon_size,
                text_left,
                max_text_width,
                egui::Color32::from_rgb(30, 58, 95),
                egui::Color32::from_rgb(100, 160, 255),
                "✎",
                title,
                progress
                    .current_name
                    .filter(|name| !name.is_empty())
                    .map(|name| truncate_tail(&name, 35)),
                Some((completed, progress.total)),
                false,
            );
        }
    }

    let bulk_progress = app
        .bulk_thumbnail_progress
        .lock()
        .ok()
        .and_then(|g| g.clone());
    let bulk_is_scanning = app
        .bulk_thumbnail_scanning
        .load(std::sync::atomic::Ordering::Relaxed);
    let bulk_total = app
        .bulk_thumbnail_total
        .load(std::sync::atomic::Ordering::Relaxed);
    let bulk_completed = app
        .bulk_thumbnail_completed
        .load(std::sync::atomic::Ordering::Relaxed)
        .min(bulk_total);

    if let Some(progress) = bulk_progress
        .filter(|_| bulk_is_scanning || (bulk_total > 0 && bulk_completed < bulk_total))
    {
        needs_repaint = true;
        let progress_toast_height = 62.0;
        let toast_y = screen.max.y - y_offset - progress_toast_height;
        y_offset += progress_toast_height + padding;

        let root_display = truncate_end(&progress.root_name, 18);
        let title = if bulk_total > 0 {
            format!(
                "{} - {} ({}/{})",
                t!("status_bar.bulk_thumbnails_extracting"),
                root_display,
                bulk_completed,
                bulk_total,
            )
        } else {
            format!(
                "{} - {}",
                t!("status_bar.bulk_thumbnails_extracting"),
                root_display,
            )
        };

        render_progress_toast(
            ctx,
            egui::Id::new("bulk_thumbnail_progress_toast"),
            egui::pos2(base_x, toast_y),
            toast_width,
            progress_toast_height,
            inner_pad,
            icon_size,
            text_left,
            max_text_width,
            egui::Color32::from_rgb(30, 58, 95),
            egui::Color32::from_rgb(100, 160, 255),
            "🖼",
            title,
            (!progress.current_file.is_empty()).then(|| truncate_tail(&progress.current_file, 35)),
            if bulk_total > 0 {
                Some((bulk_completed, bulk_total))
            } else {
                None
            },
            false,
        );
    }

    if !app.notifications.is_empty() {
        needs_repaint = true;

        for (i, notification) in app.notifications.active().iter().enumerate() {
            let fade = notification.remaining_fraction();
            let alpha = if fade < 0.2 { fade / 0.2 } else { 1.0 };

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
                bg_color.r(),
                bg_color.g(),
                bg_color.b(),
                (alpha * 240.0) as u8,
            );

            egui::Area::new(egui::Id::new(format!("toast_{}", i)))
                .fixed_pos(egui::pos2(base_x, toast_y))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let rect = egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(toast_width, toast_height),
                    );

                    ui.painter().rect_filled(rect, 8.0, bg);

                    let icon_color = egui::Color32::from_rgba_unmultiplied(
                        accent.r(),
                        accent.g(),
                        accent.b(),
                        (alpha * 255.0) as u8,
                    );
                    ui.painter().text(
                        rect.min + egui::vec2(inner_pad, (toast_height - icon_size) / 2.0),
                        egui::Align2::LEFT_TOP,
                        notification.level.icon(),
                        egui::FontId::proportional(icon_size),
                        icon_color,
                    );

                    let text_color =
                        egui::Color32::from_rgba_unmultiplied(230, 230, 230, (alpha * 255.0) as u8);
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
