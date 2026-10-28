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
            if ui
                .add_enabled(
                    !busy && valid,
                    egui::Button::new(egui::RichText::new(t!("export_listing.export")).strong())
                        .min_size(egui::vec2(120.0, 36.0)),
                )
                .clicked()
            {
                action = Action::Export;
            }
            if busy {
                if ui
                    .add(
                        egui::Button::new(t!("export_listing.cancel"))
                            .min_size(egui::vec2(100.0, 36.0)),
                    )
                    .clicked()
                {
                    action = Action::Cancel;
                }
            } else if ui
                .add(
                    egui::Button::new(t!("export_listing.close")).min_size(egui::vec2(100.0, 36.0)),
                )
                .clicked()
            {
                action = Action::Close;
            }
            action
        })
        .inner
    })
}
