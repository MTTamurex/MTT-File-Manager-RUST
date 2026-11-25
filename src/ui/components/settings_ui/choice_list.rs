//! Radio-style option list for the settings window.

use super::{accent_tint, hover_fill, paint_focus_outline, radio_arrow_target, OPTION_ROW_HEIGHT};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, WidgetInfo, WidgetType,
};

const WARNING_ICON_SIZE: f32 = 15.0;

/// Vertical list of radio-style option rows.
/// Returns the index of the clicked option, if any.
pub fn choice_list(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Option<usize> {
    choice_list_with_warnings(ui, labels, selected, &[])
}

/// [`choice_list`] where `warnings[i]`, when set, adds a warning icon after
/// option `i`'s label; hovering the icon shows the text as a tooltip.
pub fn choice_list_with_warnings(
    ui: &mut egui::Ui,
    labels: &[&str],
    selected: usize,
    warnings: &[Option<&str>],
) -> Option<usize> {
    let dark_mode = ui.visuals().dark_mode;
    let enabled = ui.is_enabled();
    let disabled_visuals = ui.visuals().widgets.noninteractive;
    let mut activated = None;
    let mut focused = None;
    let mut option_ids = Vec::with_capacity(labels.len());

    for (i, label) in labels.iter().enumerate() {
        let is_selected = i == selected;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), OPTION_ROW_HEIGHT),
            Sense::click(),
        );
        let response = response.on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        });
        option_ids.push(response.id);
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::RadioButton,
                ui.is_enabled(),
                is_selected,
                *label,
            )
        });

        let text_color = if enabled {
            theme::text_color(ui.ctx())
        } else {
            disabled_visuals
                .fg_stroke
                .color
                .gamma_multiply(ui.visuals().disabled_alpha)
        };
        let label_galley = ui.painter().layout_no_wrap(
            (*label).to_string(),
            egui::FontId::proportional(13.0),
            text_color,
        );
        let label_pos = Pos2::new(
            rect.min.x + 36.0,
            rect.center().y - label_galley.size().y / 2.0,
        );
        let warning_icon = warnings.get(i).copied().flatten().map(|warning| {
            let icon_rect = Rect::from_center_size(
                Pos2::new(
                    label_pos.x + label_galley.size().x + 6.0 + WARNING_ICON_SIZE / 2.0,
                    rect.center().y,
                ),
                Vec2::splat(WARNING_ICON_SIZE),
            );
            ui.interact(icon_rect, response.id.with("warning"), Sense::hover())
                .on_hover_text(warning);
            icon_rect
        });

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if !enabled {
                painter.rect_filled(rect, 8.0, ui.visuals().widgets.inactive.bg_fill);
                painter.rect_stroke(
                    rect,
                    8.0,
                    ui.visuals().widgets.inactive.bg_stroke,
                    StrokeKind::Inside,
                );
            } else if is_selected {
                painter.rect_filled(rect, 8.0, accent_tint(ui.ctx()));
                painter.rect_stroke(
                    rect,
                    8.0,
                    Stroke::new(1.0, theme::accent_color(ui.ctx())),
                    StrokeKind::Inside,
                );
            } else if response.hovered() || response.has_focus() {
                painter.rect_filled(rect, 8.0, hover_fill(ui.ctx()));
                painter.rect_stroke(
                    rect,
                    8.0,
                    Stroke::new(
                        1.0,
                        if dark_mode {
                            Color32::from_white_alpha(25)
                        } else {
                            Color32::from_black_alpha(25)
                        },
                    ),
                    StrokeKind::Inside,
                );
            } else {
                painter.rect_stroke(
                    rect,
                    8.0,
                    Stroke::new(
                        1.0,
                        if dark_mode {
                            theme::palette::border(ui.ctx(), Color32::from_gray(70))
                        } else {
                            Color32::from_gray(215)
                        },
                    ),
                    StrokeKind::Inside,
                );
            }

            let radio_center = Pos2::new(rect.min.x + 18.0, rect.center().y);
            if !enabled {
                let muted = disabled_visuals
                    .fg_stroke
                    .color
                    .gamma_multiply(ui.visuals().disabled_alpha);
                painter.circle_stroke(radio_center, 7.0, Stroke::new(1.5, muted));
                if is_selected {
                    painter.circle_filled(radio_center, 3.5, muted);
                }
            } else if is_selected {
                painter.circle_stroke(
                    radio_center,
                    7.0,
                    Stroke::new(1.5, theme::accent_color(ui.ctx())),
                );
                painter.circle_filled(radio_center, 3.5, theme::accent_color(ui.ctx()));
            } else {
                painter.circle_stroke(
                    radio_center,
                    7.0,
                    Stroke::new(
                        1.5,
                        if dark_mode {
                            Color32::from_gray(140)
                        } else {
                            Color32::from_gray(120)
                        },
                    ),
                );
            }
            painter.galley(label_pos, label_galley, text_color);
            if let Some(icon_rect) = warning_icon {
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    theme::ICON_WARNING,
                    egui::FontId::new(WARNING_ICON_SIZE, egui::FontFamily::Name("icons".into())),
                    ui.visuals().warn_fg_color,
                );
            }
            if enabled && response.has_focus() {
                paint_focus_outline(ui, rect, 8.0);
            }
        }

        if response.clicked() && !is_selected {
            activated = Some(i);
        }
        if response.has_focus() {
            focused = Some(i);
        }
        ui.add_space(6.0);
    }

    if enabled {
        if let Some(focused) = focused {
            if let Some(target) = radio_arrow_target(
                ui,
                focused,
                labels.len(),
                &[egui::Key::ArrowUp, egui::Key::ArrowLeft],
                &[egui::Key::ArrowDown, egui::Key::ArrowRight],
            ) {
                ui.memory_mut(|memory| memory.request_focus(option_ids[target]));
                return (target != selected).then_some(target);
            }
        }
    }
    activated
}
