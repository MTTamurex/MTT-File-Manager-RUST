//! The standard ComboBox hard-codes an animated ScrollArea. Own the popup so
//! font/size changes and keyboard jumps can reposition its contents immediately.

use eframe::egui;

fn popup_id(button_id: egui::Id) -> egui::Id {
    button_id.with("installed_font_menu")
}

pub(super) fn is_open(ctx: &egui::Context, button_id: egui::Id) -> bool {
    egui::Popup::is_id_open(ctx, popup_id(button_id))
}

pub(super) fn show<R>(
    ui: &mut egui::Ui,
    label: &str,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let id = ui.make_persistent_id("ui_font_family_selector");
    let response = button(ui, id, label, is_open(ui.ctx(), id));
    let inner = egui::Popup::menu(&response)
        .id(popup_id(id))
        .width(response.rect.width())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("installed_font_scroll")
                .max_height(260.0)
                .animated(false)
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    contents(ui)
                })
                .inner
        })
        .map(|output| output.inner);
    egui::InnerResponse { inner, response }
}

fn button(ui: &mut egui::Ui, id: egui::Id, label: &str, open: bool) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let icon_size = ui.spacing().icon_width;
    let wrap_mode = ui.wrap_mode();
    let wrap_width = if wrap_mode == egui::TextWrapMode::Extend {
        f32::INFINITY
    } else {
        (ui.available_width() - 2.0 * padding.x - ui.spacing().icon_spacing - icon_size).max(0.0)
    };
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(wrap_mode),
        wrap_width,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(
        (galley.size().x + ui.spacing().icon_spacing + icon_size + 2.0 * padding.x)
            .max(ui.spacing().combo_width),
        (galley.size().y.max(icon_size) + 2.0 * padding.y).max(ui.spacing().interact_size.y),
    );
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id, egui::Sense::click());
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::new(egui::WidgetType::ComboBox);
        info.enabled = ui.is_enabled();
        info.current_text_value = Some(label.to_owned());
        info
    });
    if ui.is_rect_visible(rect) {
        let visuals = if open {
            &ui.visuals().widgets.open
        } else {
            ui.style().interact(&response)
        };
        ui.painter().rect(
            rect.expand(visuals.expansion),
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let text_pos = egui::pos2(
            rect.left() + padding.x,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(text_pos, galley, visuals.text_color());
        let icon = egui::Rect::from_center_size(
            egui::pos2(rect.right() - padding.x - icon_size / 2.0, rect.center().y),
            egui::vec2(icon_size * 0.7, icon_size * 0.45),
        );
        ui.painter().add(egui::Shape::convex_polygon(
            vec![icon.left_top(), icon.right_top(), icon.center_bottom()],
            visuals.fg_stroke.color,
            egui::Stroke::NONE,
        ));
    }
    response
}
