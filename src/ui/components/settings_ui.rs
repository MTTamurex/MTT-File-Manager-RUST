//! Shared UI kit for the settings window: section headers, sidebar navigation
//! items, segmented/radio choice controls and switch toggles with a modern,
//! Windows 11-inspired look.

use crate::ui::theme;
use eframe::egui::{
    self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, WidgetInfo, WidgetType,
};

const NAV_ITEM_HEIGHT: f32 = 34.0;
const SEGMENT_HEIGHT: f32 = 30.0;
const OPTION_ROW_HEIGHT: f32 = 36.0;

fn accent_tint(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgba_unmultiplied(0, 120, 215, 55)
    } else {
        Color32::from_rgba_unmultiplied(0, 120, 215, 24)
    }
}

fn hover_fill(dark_mode: bool) -> Color32 {
    if dark_mode {
        theme::color_dark_hover()
    } else {
        theme::color_hover()
    }
}

fn paint_focus_outline(ui: &egui::Ui, rect: Rect, radius: f32) {
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::COLOR_ACCENT),
        StrokeKind::Inside,
    );
}

fn radio_arrow_target(
    ui: &mut egui::Ui,
    focused: usize,
    option_count: usize,
    previous_keys: &[egui::Key],
    next_keys: &[egui::Key],
) -> Option<usize> {
    if option_count == 0 {
        return None;
    }

    let (previous, next) = ui.input_mut(|input| {
        let previous = previous_keys.iter().fold(false, |pressed, &key| {
            input.consume_key(egui::Modifiers::NONE, key) || pressed
        });
        let next = next_keys.iter().fold(false, |pressed, &key| {
            input.consume_key(egui::Modifiers::NONE, key) || pressed
        });
        (previous, next)
    });
    if previous || next {
        ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }

    match (previous, next) {
        (true, false) => Some((focused + option_count - 1) % option_count),
        (false, true) => Some((focused + 1) % option_count),
        _ => None,
    }
}

/// Section title + description with consistent typography.
pub fn section_header(ui: &mut egui::Ui, title: &str, description: &str) {
    let dark_mode = ui.visuals().dark_mode;
    ui.label(
        RichText::new(title)
            .size(16.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(description)
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(16.0);
}

/// Sidebar navigation entry with rounded hover/selection and accent indicator.
/// Returns true when clicked.
pub fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let dark_mode = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), NAV_ITEM_HEIGHT),
        Sense::click(),
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 6.0, accent_tint(dark_mode));
            let indicator = Rect::from_min_size(
                Pos2::new(rect.min.x, rect.center().y - 8.0),
                Vec2::new(3.0, 16.0),
            );
            painter.rect_filled(indicator, 1.5, theme::COLOR_ACCENT);
        } else if response.hovered() || response.has_focus() {
            painter.rect_filled(rect, 6.0, hover_fill(dark_mode));
        }
        let mut text = RichText::new(label)
            .size(13.0)
            .color(theme::text_color(dark_mode));
        if selected {
            text = text.strong();
        }
        let galley = egui::WidgetText::from(text).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::FontId::proportional(13.0),
        );
        let text_pos = Pos2::new(rect.min.x + 12.0, rect.center().y - 0.5 * galley.size().y);
        ui.painter()
            .galley(text_pos, galley, theme::text_color(dark_mode));
        if response.has_focus() {
            paint_focus_outline(ui, rect, 6.0);
        }
    }

    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, label));
    response.clicked()
}

/// Horizontal segmented control (Windows 11 style).
/// Returns the index of the clicked segment, if any.
pub fn segmented_choice(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let dark_mode = ui.visuals().dark_mode;
    let font_id = egui::FontId::proportional(13.0);
    let text_color = theme::text_color(dark_mode);

    let mut max_text_width: f32 = 0.0;
    for label in labels {
        let galley = ui
            .painter()
            .layout_no_wrap(label.to_string(), font_id.clone(), text_color);
        max_text_width = max_text_width.max(galley.rect.width());
    }
    let pad = 3.0;
    let seg_w = max_text_width + 24.0;
    let size = Vec2::new(
        seg_w * labels.len() as f32 + pad * 2.0,
        SEGMENT_HEIGHT + pad * 2.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let mut activated = None;
    let mut focused = None;

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let container = if dark_mode {
            Color32::from_white_alpha(12)
        } else {
            Color32::from_black_alpha(15)
        };
        painter.rect_filled(rect, 7.0, container);
    }

    for (i, label) in labels.iter().enumerate() {
        let seg_rect = Rect::from_min_size(
            Pos2::new(rect.min.x + pad + i as f32 * seg_w, rect.min.y + pad),
            Vec2::new(seg_w, SEGMENT_HEIGHT),
        );
        let segment_response = ui
            .interact(seg_rect, response.id.with(i), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let is_selected = i == selected;
        segment_response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::RadioButton,
                ui.is_enabled(),
                is_selected,
                *label,
            )
        });

        if ui.is_rect_visible(seg_rect) {
            let painter = ui.painter();
            if is_selected {
                let fill = if dark_mode {
                    Color32::from_gray(69)
                } else {
                    Color32::WHITE
                };
                let stroke = Stroke::new(
                    1.0,
                    if dark_mode {
                        Color32::from_white_alpha(18)
                    } else {
                        Color32::from_black_alpha(20)
                    },
                );
                painter.rect(seg_rect, 5.0, fill, stroke, StrokeKind::Inside);
            } else if segment_response.hovered() || segment_response.has_focus() {
                painter.rect_filled(
                    seg_rect,
                    5.0,
                    if dark_mode {
                        Color32::from_white_alpha(8)
                    } else {
                        Color32::from_black_alpha(8)
                    },
                );
            }
            painter.text(
                seg_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font_id.clone(),
                text_color,
            );
            if segment_response.has_focus() {
                paint_focus_outline(ui, seg_rect, 5.0);
            }
        }

        if segment_response.clicked() && !is_selected {
            activated = Some(i);
        }
        if segment_response.has_focus() {
            focused = Some(i);
        }
    }

    if let Some(focused) = focused {
        if let Some(target) = radio_arrow_target(
            ui,
            focused,
            labels.len(),
            &[egui::Key::ArrowLeft],
            &[egui::Key::ArrowRight],
        ) {
            ui.memory_mut(|memory| memory.request_focus(response.id.with(target)));
            return (target != selected).then_some(target);
        }
    }
    activated
}

/// Vertical list of radio-style option rows.
/// Returns the index of the clicked option, if any.
pub fn choice_list(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let dark_mode = ui.visuals().dark_mode;
    let mut activated = None;
    let mut focused = None;
    let mut option_ids = Vec::with_capacity(labels.len());

    for (i, label) in labels.iter().enumerate() {
        let is_selected = i == selected;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), OPTION_ROW_HEIGHT),
            Sense::click(),
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        option_ids.push(response.id);
        response.widget_info(|| {
            WidgetInfo::selected(
                WidgetType::RadioButton,
                ui.is_enabled(),
                is_selected,
                *label,
            )
        });

        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if is_selected {
                painter.rect_filled(rect, 8.0, accent_tint(dark_mode));
                painter.rect_stroke(
                    rect,
                    8.0,
                    Stroke::new(1.0, theme::COLOR_ACCENT),
                    StrokeKind::Inside,
                );
            } else if response.hovered() || response.has_focus() {
                painter.rect_filled(rect, 8.0, hover_fill(dark_mode));
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
                            Color32::from_gray(70)
                        } else {
                            Color32::from_gray(215)
                        },
                    ),
                    StrokeKind::Inside,
                );
            }

            let radio_center = Pos2::new(rect.min.x + 18.0, rect.center().y);
            if is_selected {
                painter.circle_stroke(radio_center, 7.0, Stroke::new(1.5, theme::COLOR_ACCENT));
                painter.circle_filled(radio_center, 3.5, theme::COLOR_ACCENT);
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
            painter.text(
                Pos2::new(rect.min.x + 36.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(13.0),
                theme::text_color(dark_mode),
            );
            if response.has_focus() {
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
    activated
}

/// Label + switch row (Windows 11 style). Returns true when toggled.
pub fn toggle_row(ui: &mut egui::Ui, label: &str, value: &mut bool) -> bool {
    let dark_mode = ui.visuals().dark_mode;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let mut response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.text(
            Pos2::new(rect.min.x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            theme::text_color(dark_mode),
        );
        let track = Rect::from_min_size(
            Pos2::new(rect.max.x - 40.0, rect.center().y - 10.0),
            Vec2::new(40.0, 20.0),
        );
        paint_switch(
            ui,
            track,
            *value,
            response.hovered() || response.has_focus(),
        );
        if response.has_focus() {
            paint_focus_outline(ui, rect, 4.0);
        }
    }

    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), *value, label));
    response.changed()
}

/// Standalone switch control (40x20).
pub fn toggle_switch(
    ui: &mut egui::Ui,
    value: &mut bool,
    accessible_label: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 20.0), Sense::click());
    let mut response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        paint_switch(ui, rect, *value, response.hovered() || response.has_focus());
        if response.has_focus() {
            paint_focus_outline(ui, rect, 10.0);
        }
    }
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::Checkbox,
            ui.is_enabled(),
            *value,
            accessible_label,
        )
    });
    response
}

fn paint_switch(ui: &mut egui::Ui, track: Rect, on: bool, hovered: bool) {
    let dark_mode = ui.visuals().dark_mode;
    let painter = ui.painter();
    if on {
        painter.rect_filled(track, 10.0, theme::COLOR_ACCENT);
        painter.circle_filled(
            Pos2::new(track.max.x - 10.0, track.center().y),
            6.0,
            Color32::WHITE,
        );
    } else {
        let border = if hovered {
            theme::COLOR_ACCENT
        } else if dark_mode {
            Color32::from_gray(130)
        } else {
            Color32::from_gray(140)
        };
        painter.rect_filled(track, 10.0, theme::input_bg_color(dark_mode));
        painter.rect_stroke(track, 10.0, Stroke::new(1.0, border), StrokeKind::Inside);
        let knob_color = if dark_mode {
            Color32::from_gray(190)
        } else {
            Color32::from_gray(90)
        };
        painter.circle_filled(
            Pos2::new(track.min.x + 10.0, track.center().y),
            5.0,
            knob_color,
        );
    }
}

/// Text input styled for use inside cards (visible fill + border on any background).
pub fn text_edit(ui: &mut egui::Ui, width: f32, text: &mut String) -> egui::Response {
    let dark_mode = ui.visuals().dark_mode;
    ui.scope(|ui| {
        let widget = &mut ui.visuals_mut().widgets.inactive;
        widget.bg_fill = if dark_mode {
            Color32::from_gray(43)
        } else {
            Color32::from_rgb(247, 247, 247)
        };
        widget.bg_stroke = Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_gray(85)
            } else {
                Color32::from_gray(208)
            },
        );
        ui.add_sized(Vec2::new(width, 26.0), egui::TextEdit::singleline(text))
    })
    .inner
}

/// 14px strong sub-section title.
pub fn sub_header(ui: &mut egui::Ui, text: &str) {
    let dark_mode = ui.visuals().dark_mode;
    ui.label(
        RichText::new(text)
            .size(14.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
}

/// Compact rounded container for list rows (tags, organizer rules).
pub fn row_frame(dark_mode: bool) -> egui::Frame {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 8,
            bottom: 8,
        })
        .corner_radius(8.0)
        .fill(if dark_mode {
            Color32::from_gray(50)
        } else {
            Color32::WHITE
        })
        .stroke(Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_gray(65)
            } else {
                Color32::from_gray(228)
            },
        ))
}

/// Rounded card container used by settings sections.
pub fn card_frame(dark_mode: bool) -> egui::Frame {
    egui::Frame::new()
        .inner_margin(egui::Margin::same(14))
        .corner_radius(8.0)
        .fill(if dark_mode {
            Color32::from_gray(50)
        } else {
            Color32::WHITE
        })
        .stroke(Stroke::new(
            1.0,
            if dark_mode {
                Color32::from_gray(65)
            } else {
                Color32::from_gray(228)
            },
        ))
}

#[cfg(test)]
#[path = "settings_ui_tests.rs"]
mod tests;
