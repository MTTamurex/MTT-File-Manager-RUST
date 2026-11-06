use crate::ui::{components::settings_ui, theme};
use eframe::egui;
use rust_i18n::t;

pub(crate) fn card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    settings_ui::card_frame(ui.ctx())
        .inner_margin(20.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
            contents(ui);
        });
}

pub(crate) fn heading(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.add(egui::Label::new(egui::RichText::new(title).size(18.0).strong()).wrap());
    if let Some(subtitle) = subtitle {
        ui.add(
            egui::Label::new(
                egui::RichText::new(subtitle)
                    .color(theme::secondary_text_color(ui.ctx()))
                    .small(),
            )
            .wrap(),
        );
    }
    ui.add_space(16.0);
}

pub(crate) fn field(ui: &mut egui::Ui, label: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.label(
        egui::RichText::new(label)
            .small()
            .color(theme::secondary_text_color(ui.ctx())),
    );
    ui.add_space(2.0);
    contents(ui);
    ui.add_space(12.0);
}

pub(crate) fn input(ui: &mut egui::Ui, text: &mut String, width: f32) -> egui::Response {
    ui.add_sized(
        [width.max(24.0), ui.spacing().interact_size.y.max(32.0)],
        egui::TextEdit::singleline(text).margin(egui::vec2(8.0, 6.0)),
    )
}

/// Keep the save action on the name's row, instead of inserting another
/// unlabelled row between the name, color and hierarchy fields.
pub(crate) fn name_with_save(ui: &mut egui::Ui, name: &mut String) -> bool {
    ui.horizontal(|ui| {
        let label = t!("tags.save");
        let button_width = ui
            .painter()
            .layout_no_wrap(
                label.to_string(),
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().text_color(),
            )
            .size()
            .x
            + ui.spacing().button_padding.x * 2.0
            + 12.0;
        let width = ui.available_width() - button_width - ui.spacing().item_spacing.x;
        input(ui, name, width);
        ui.add_sized(
            [button_width, ui.spacing().interact_size.y.max(32.0)],
            egui::Button::new(label),
        )
        .clicked()
    })
    .inner
}

pub(crate) fn creation_actions(ui: &mut egui::Ui, label: &str, cancelled: &mut bool) -> bool {
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    ui.with_layout(
        egui::Layout::right_to_left(egui::Align::Center).with_main_wrap(true),
        |ui| {
            let create = ui.add(egui::Button::new(label).wrap()).clicked();
            *cancelled = ui.button(t!("tags.cancel")).clicked();
            create
        },
    )
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_creation_actions_fit_a_narrow_dialog() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(180.0);
            creation_actions(ui, "Create automatic tagging rule", &mut false);
            assert!(
                ui.min_rect().width() <= 181.0,
                "localized footer actions must wrap instead of expanding the dialog"
            );
        });
    }

    #[test]
    fn large_fonts_do_not_push_name_actions_outside_narrow_cards() {
        for width in [180.0, 260.0, 440.0] {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| {
                for font in style.text_styles.values_mut() {
                    font.size *= 3.0;
                }
            });
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                card(ui, |ui| {
                    name_with_save(ui, &mut String::from("Tag"));
                });
                assert!(
                    ui.min_rect().width() <= width + 1.0,
                    "name input and save action must fit the card width"
                );
            });
        }
    }

    #[test]
    fn editor_fields_have_clear_spacing_and_fit_inside_the_card() {
        for width in [260.0, 440.0] {
            let ctx = egui::Context::default();
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(width);
                let mut name = "Example tag".to_owned();
                let mut rows = Vec::new();
                card(ui, |ui| {
                    heading(ui, "Details", Some("Item count"));
                    field(ui, "Name", |ui| {
                        let top = ui.cursor().top();
                        name_with_save(ui, &mut name);
                        rows.push((top, ui.min_rect().bottom()));
                    });
                    field(ui, "Color", |ui| {
                        let rect = ui.button("Color picker").rect;
                        rows.push((rect.top(), rect.bottom()));
                    });
                    field(ui, "Parent", |ui| {
                        let rect = egui::ComboBox::from_id_salt("parent")
                            .width(ui.available_width())
                            .truncate()
                            .selected_text("No parent")
                            .show_ui(ui, |_| {})
                            .response
                            .rect;
                        rows.push((rect.top(), rect.bottom()));
                    });
                });
                assert!(ui.min_rect().width() <= width + 1.0);
                assert!(rows[1].0 - rows[0].1 >= 12.0);
                assert!(rows[2].0 - rows[1].1 >= 12.0);
            });
        }
    }
}
