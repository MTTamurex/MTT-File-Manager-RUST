use crate::ui::theme;
use eframe::egui::{self, Color32, Stroke, Vec2};

use super::TabBarAction;

/// Render window control buttons (Minimize, Maximize, Close)
/// Used for borderless window mode where native decorations are disabled.
pub(super) fn render_window_controls(
    ui: &mut egui::Ui,
    _frame: &mut eframe::Frame,
    btn_width: f32,
    btn_height: f32,
    action: &mut TabBarAction,
) {
    let is_dark = ui.visuals().dark_mode;
    let text_color = if is_dark {
        Color32::from_rgb(220, 220, 220)
    } else {
        Color32::from_rgb(30, 30, 30)
    };
    let normal_bg = if is_dark {
        Color32::from_rgb(30, 30, 30)
    } else {
        Color32::from_rgb(230, 230, 230)
    };
    let hover_bg = if is_dark {
        theme::color_dark_hover()
    } else {
        theme::color_hover()
    };
    let close_hover_bg = Color32::from_rgb(232, 17, 35);

    ui.spacing_mut().item_spacing.x = 0.0;

    let is_maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));

    let (min_rect, min_response) =
        ui.allocate_exact_size(Vec2::new(btn_width, btn_height), egui::Sense::click());

    if min_response.clicked() {
        *action = TabBarAction::Minimize;
    }

    let min_bg = if min_response.hovered() {
        hover_bg
    } else {
        normal_bg
    };
    ui.painter().rect_filled(min_rect, 0.0, min_bg);

    let min_center = min_rect.center();
    ui.painter().line_segment(
        [
            min_center + Vec2::new(-5.0, 0.0),
            min_center + Vec2::new(5.0, 0.0),
        ],
        Stroke::new(1.0, text_color),
    );

    let (max_rect, max_response) =
        ui.allocate_exact_size(Vec2::new(btn_width, btn_height), egui::Sense::click());

    if max_response.clicked() {
        *action = TabBarAction::ToggleMaximize;
    }

    let max_bg = if max_response.hovered() {
        hover_bg
    } else {
        normal_bg
    };
    ui.painter().rect_filled(max_rect, 0.0, max_bg);

    let max_center = max_rect.center();
    let icon_size = 10.0;

    if is_maximized {
        let offset = 2.0;
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(
                max_center + Vec2::new(-offset, offset),
                Vec2::splat(icon_size - 2.0),
            ),
            0.0,
            Stroke::new(1.0, text_color),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(
                max_center + Vec2::new(offset, -offset),
                Vec2::splat(icon_size - 2.0),
            ),
            0.0,
            Stroke::new(1.0, text_color),
            egui::StrokeKind::Inside,
        );
    } else {
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(max_center, Vec2::splat(icon_size)),
            0.0,
            Stroke::new(1.0, text_color),
            egui::StrokeKind::Inside,
        );
    }

    let (close_rect, close_response) =
        ui.allocate_exact_size(Vec2::new(btn_width, btn_height), egui::Sense::click());

    if close_response.clicked() {
        *action = TabBarAction::CloseApp;
    }

    let close_bg = if close_response.hovered() {
        close_hover_bg
    } else {
        normal_bg
    };
    ui.painter().rect_filled(close_rect, 0.0, close_bg);

    let close_center = close_rect.center();
    let x_size = 10.0;
    let x_color = if close_response.hovered() {
        Color32::WHITE
    } else {
        text_color
    };
    let x_stroke = Stroke::new(1.0, x_color);

    ui.painter().line_segment(
        [
            close_center + Vec2::new(-x_size / 2.0, -x_size / 2.0),
            close_center + Vec2::new(x_size / 2.0, x_size / 2.0),
        ],
        x_stroke,
    );
    ui.painter().line_segment(
        [
            close_center + Vec2::new(x_size / 2.0, -x_size / 2.0),
            close_center + Vec2::new(-x_size / 2.0, x_size / 2.0),
        ],
        x_stroke,
    );
}
