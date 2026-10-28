use eframe::egui;
use rust_i18n::t;

pub(super) enum Action {
    None,
    Export,
    Cancel,
    Close,
}

pub(super) fn render(ui: &mut egui::Ui, busy: bool, valid: bool) -> egui::InnerResponse<Action> {
    // A horizontal row bounds the available height before right-aligning.
    // Using with_layout directly in the modal fills its remaining height.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut action = Action::None;
            let export_text: egui::WidgetText = egui::RichText::new(t!("export_listing.export"))
                .strong()
                .into();
            let secondary_text: egui::WidgetText = if busy {
                t!("export_listing.cancel")
            } else {
                t!("export_listing.close")
            }
            .into();
            let padding = ui.spacing().button_padding * 2.0;
            let button_size = [export_text.clone(), secondary_text.clone()]
                .into_iter()
                .map(|text| {
                    text.into_galley(
                        ui,
                        Some(egui::TextWrapMode::Extend),
                        f32::INFINITY,
                        egui::TextStyle::Button,
                    )
                    .size()
                        + padding
                })
                .fold(
                    egui::vec2(120.0, 36.0).max(ui.spacing().interact_size),
                    |size, measured| size.max(measured),
                );
            if ui
                .add_enabled(
                    !busy && valid,
                    egui::Button::new(export_text).min_size(button_size),
                )
                .clicked()
            {
                action = Action::Export;
            }
            if ui
                .add(egui::Button::new(secondary_text).min_size(button_size))
                .clicked()
            {
                action = if busy { Action::Cancel } else { Action::Close };
            }
            action
        })
        .inner
    })
}
