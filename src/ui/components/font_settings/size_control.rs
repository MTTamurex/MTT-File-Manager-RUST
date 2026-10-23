use crate::ui::fonts::sizing;
use eframe::egui;
use rust_i18n::t;

/// Keep the current font metrics stable while dragging. Otherwise the slider
/// and its label would move underneath the pointer as their text changes size.
pub(super) fn render(
    ui: &mut egui::Ui,
    family: &str,
    scale: f32,
    enabled: bool,
) -> (Option<f32>, egui::Response) {
    let draft_id = ui.make_persistent_id(("font_scale_percent", family));
    let draft = ui.ctx().data(|data| data.get_temp::<f32>(draft_id));
    let mut percent = draft.unwrap_or(scale * 100.0);
    let mut reset = false;
    let response = ui
        .add_enabled_ui(enabled, |ui| {
            ui.label(t!("settings.ui_font_scale"));
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::Slider::new(&mut percent, sizing::MIN * 100.0..=sizing::MAX * 100.0)
                        .step_by(5.0)
                        .suffix("%"),
                );
                reset = ui
                    .button("100%")
                    .on_hover_text(t!("settings.ui_font_scale_reset"))
                    .clicked();
                response
            })
            .inner
        })
        .inner;
    if !enabled {
        ui.ctx().data_mut(|data| data.remove::<f32>(draft_id));
        return (None, response);
    }
    if reset {
        ui.ctx().data_mut(|data| data.remove::<f32>(draft_id));
        return (Some(sizing::DEFAULT), response);
    }
    let value = if response.is_pointer_button_down_on() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(draft_id, percent));
        None
    } else {
        ui.ctx().data_mut(|data| data.remove::<f32>(draft_id));
        (response.changed() || draft.is_some()).then_some(percent / 100.0)
    };
    (value, response)
}

#[cfg(test)]
mod tests;
