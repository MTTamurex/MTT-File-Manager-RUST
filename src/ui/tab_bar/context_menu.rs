use crate::tabs::actions::TabAction;
use eframe::egui;
use rust_i18n::t;

/// Both tab strips expose the same commands without selecting the clicked tab.
pub(crate) fn tab_context_menu(
    response: &egui::Response,
    pinned: bool,
    can_duplicate: bool,
    can_close_left: bool,
    can_close_right: bool,
    quick_access_action: Option<TabAction>,
) -> Option<TabAction> {
    let mut action = None;
    response.context_menu(|ui| {
        for (command, enabled, label) in [
            (TabAction::Close, !pinned, t!("tabs.close")),
            (TabAction::Duplicate, can_duplicate, t!("tabs.duplicate")),
            (TabAction::CloseLeft, can_close_left, t!("tabs.close_left")),
            (
                TabAction::CloseRight,
                can_close_right,
                t!("tabs.close_right"),
            ),
            (
                TabAction::TogglePin,
                true,
                if pinned {
                    t!("tabs.unpin")
                } else {
                    t!("tabs.pin")
                },
            ),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                action = Some(command);
                ui.close();
            }
        }
        if let Some(command @ (TabAction::PinQuickAccess | TabAction::UnpinQuickAccess)) =
            quick_access_action
        {
            ui.separator();
            let label = if command == TabAction::UnpinQuickAccess {
                t!("context_menu.unpin_quick_access")
            } else {
                t!("context_menu.pin_quick_access")
            };
            if ui.button(label).clicked() {
                action = Some(command);
                ui.close();
            }
        }
    });
    action
}

pub(crate) fn close_control(
    ui: &egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    pinned: bool,
    hover_bg: egui::Color32,
    text: egui::Color32,
    inactive: egui::Color32,
) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if pinned {
        let pin_color = if response.hovered() {
            egui::Color32::from_rgb(220, 60, 60)
        } else {
            egui::Color32::from_gray(140)
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "📌",
            egui::FontId::proportional(13.0),
            pin_color,
        );
        response.clone().on_hover_text(t!("tabs.unpin"));
    } else {
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(4), hover_bg);
        }
        let stroke = egui::Stroke::new(1.5, if response.hovered() { text } else { inactive });
        let center = rect.center();
        let radius = rect.width() * 0.25;
        for direction in [-1.0, 1.0] {
            ui.painter().line_segment(
                [
                    center + egui::vec2(-radius, direction * radius),
                    center + egui::vec2(radius, -direction * radius),
                ],
                stroke,
            );
        }
    }
    response
}
