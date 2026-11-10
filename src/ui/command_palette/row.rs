use crate::app::command_palette::search::SearchEntry;
use eframe::egui;

pub(super) fn render(
    ui: &mut egui::Ui,
    entry: &SearchEntry,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    // Always reserve the right-side spacer, including for commands with no shortcut.
    // Otherwise egui centers the label and every row starts at a different position.
    let mut shortcut_font = egui::TextStyle::Body.resolve(ui.style());
    shortcut_font.size *= 0.9;
    let label_font = egui::TextStyle::Button.resolve(ui.style());
    let text_width = |text: &str, font: &egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), ui.visuals().text_color())
            .size()
            .x
    };
    let required_width = text_width(&entry.label, &label_font)
        + text_width(&entry.inputs, &shortcut_font)
        + ui.spacing().button_padding.x * 2.0
        + 24.0;
    let button = if !entry.inputs.is_empty() && required_width > ui.available_width() {
        // A long right-hand shortcut can otherwise leave less than one glyph of
        // space for the label. Stack both texts without shrinking either font.
        let mut text = egui::text::LayoutJob::default();
        text.append(
            &format!("{}\n", entry.label),
            0.0,
            egui::TextFormat {
                font_id: label_font,
                color: egui::Color32::PLACEHOLDER,
                ..Default::default()
            },
        );
        text.append(
            &entry.inputs,
            0.0,
            egui::TextFormat {
                font_id: shortcut_font,
                color: egui::Color32::PLACEHOLDER,
                ..Default::default()
            },
        );
        egui::Button::selectable(selected, text).shortcut_text("")
    } else {
        egui::Button::selectable(selected, &entry.label)
            .shortcut_text(egui::RichText::new(&entry.inputs).font(shortcut_font))
    }
    .wrap();
    ui.scope(|ui| {
        // A stationary pointer must not add a second highlight after keyboard navigation.
        if !selected {
            let mut unselected = ui.visuals().widgets.inactive;
            unselected.bg_fill = egui::Color32::TRANSPARENT;
            unselected.weak_bg_fill = egui::Color32::TRANSPARENT;
            unselected.bg_stroke = egui::Stroke::NONE;
            ui.visuals_mut().widgets.hovered = unselected;
            ui.visuals_mut().widgets.active = unselected;
        }
        ui.add_enabled_ui(enabled, |ui| {
            ui.add_sized([ui.available_width(), 32.0], button)
        })
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command_palette::catalog::Command;

    #[test]
    fn labels_share_a_left_edge_with_or_without_shortcuts_and_when_disabled() {
        for scale in [1.0, 1.5] {
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| {
                for font in style.text_styles.values_mut() {
                    font.size *= scale;
                }
            });
            let mut bounds = Vec::new();
            let mut expected_shortcut_size = 0.0;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(640.0);
                expected_shortcut_size = egui::TextStyle::Body.resolve(ui.style()).size * 0.9;
                for (label, inputs, enabled) in [
                    ("Clear selection", "", true),
                    ("Go back", "Alt+ArrowLeft  ·  Mouse 4", true),
                    ("Switch to Grid view", "Cycle: Ctrl+Wheel ↕", false),
                ] {
                    let entry = SearchEntry {
                        command: Command::ClearSelection,
                        label: label.into(),
                        inputs: inputs.into(),
                    };
                    bounds.push(render(ui, &entry, false, enabled).rect);
                }
            });
            let mut positions = Vec::new();
            let mut shortcut_sizes = Vec::new();
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    if ["Clear selection", "Go back", "Switch to Grid view"]
                        .contains(&text.galley.text())
                    {
                        positions.push(text.pos.x);
                    }
                    if ["Alt+ArrowLeft  ·  Mouse 4", "Cycle: Ctrl+Wheel ↕"]
                        .contains(&text.galley.text())
                    {
                        shortcut_sizes.push(text.galley.job.sections[0].format.font_id.size);
                    }
                }
            }
            assert_eq!(positions.len(), 3);
            assert_eq!(shortcut_sizes.len(), 2);
            assert!(shortcut_sizes
                .iter()
                .all(|size| (*size - expected_shortcut_size).abs() < 0.1));
            for (position, rect) in positions.iter().zip(bounds.iter()) {
                assert!(
                    (*position - positions[0]).abs() < 1.0,
                    "labels must not be centered independently: {positions:?}"
                );
                assert!(*position - rect.left() < 16.0);
                assert!(rect.width() <= 641.0);
            }
        }
    }
}
