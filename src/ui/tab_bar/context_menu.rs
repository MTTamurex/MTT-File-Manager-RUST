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

pub(crate) fn paint_pin(ui: &egui::Ui, rect: egui::Rect) {
    let center = rect.center();
    let stroke = egui::Stroke::new(1.5, ui.visuals().text_color());
    ui.painter().line_segment(
        [
            center + egui::vec2(-3.0, -4.0),
            center + egui::vec2(3.0, -4.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(-2.0, -4.0),
            center + egui::vec2(-2.0, 0.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(2.0, -4.0),
            center + egui::vec2(2.0, 0.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            center + egui::vec2(-4.0, 1.0),
            center + egui::vec2(4.0, 1.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [center + egui::vec2(0.0, 1.0), center + egui::vec2(0.0, 6.0)],
        stroke,
    );
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
    let response = ui.interact(
        rect,
        id,
        if pinned {
            egui::Sense::hover()
        } else {
            egui::Sense::click()
        },
    );
    if pinned {
        paint_pin(ui, rect);
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
