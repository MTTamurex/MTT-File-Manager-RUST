use crate::app::ImageViewerApp;
use crate::ui::theme;
use eframe::egui::{self, Color32, RichText};
use rust_i18n::t;

const TITLE_BAR_HEIGHT: f32 = 48.0;

/// Custom title bar: settings icon + left-aligned title, modern close button.
pub(super) fn render_settings_title_bar(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    dark_mode: bool,
    keep_open: &mut bool,
) {
    let (header_rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TITLE_BAR_HEIGHT),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(header_rect) {
        return;
    }

    let icon_color = if dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };
    if let Some(texture) = app
        .svg_icon_manager
        .get_icon(ui.ctx(), "settings", 40, icon_color)
    {
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(header_rect.min.x + 26.0, header_rect.center().y),
            egui::vec2(20.0, 20.0),
        );
        ui.painter().image(
            texture.id(),
            icon_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let title_rect = egui::Rect::from_min_max(
        egui::pos2(header_rect.min.x + 46.0, header_rect.center().y - 10.0),
        egui::pos2(header_rect.max.x - 52.0, header_rect.center().y + 10.0),
    );
    let title_galley = egui::WidgetText::from(
        RichText::new(t!("settings.window_title").to_string())
            .size(15.0)
            .strong()
            .color(theme::text_color(ui.ctx())),
    )
    .into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::FontId::proportional(15.0),
    );
    let title_pos = egui::pos2(
        title_rect.min.x,
        header_rect.center().y - 0.5 * title_galley.size().y,
    );
    ui.painter()
        .galley(title_pos, title_galley, theme::text_color(ui.ctx()));

    let close_rect = egui::Rect::from_min_size(
        egui::pos2(header_rect.max.x - 44.0, header_rect.center().y - 14.0),
        egui::vec2(32.0, 28.0),
    );
    let close_resp = ui.interact(
        close_rect,
        ui.id().with("settings_close_button"),
        egui::Sense::click(),
    );
    let hovered = close_resp.hovered() || close_resp.has_focus();
    ui.painter().rect_filled(
        close_rect,
        6.0,
        if hovered {
            Color32::from_rgb(196, 43, 28)
        } else {
            Color32::TRANSPARENT
        },
    );
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(16.0),
        if hovered {
            Color32::WHITE
        } else {
            theme::secondary_text_color(ui.ctx())
        },
    );
    let close_resp = close_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    close_resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            t!("settings.close"),
        )
    });
    if close_resp.clicked() {
        *keep_open = false;
    }
}
