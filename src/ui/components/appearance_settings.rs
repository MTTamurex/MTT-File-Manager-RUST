use crate::app::navigation_state::ThemeMode;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub fn render_appearance_settings_section(
    ui: &mut egui::Ui,
    theme_mode: &mut ThemeMode,
    interface_scale: &mut f32,
) -> bool {
    let mut changed = false;

    settings_ui::section_header(
        ui,
        &t!("settings.appearance"),
        &t!("settings.appearance_description"),
    );

    let modes: [(ThemeMode, &str); 5] = [
        (ThemeMode::Light, &t!("settings.theme_light")),
        (ThemeMode::Dark, &t!("settings.theme_dark")),
        (ThemeMode::DarkDeep, &t!("settings.theme_dark_deep")),
        (ThemeMode::GitHubDark, &t!("settings.theme_github_dark")),
        (ThemeMode::System, &t!("settings.theme_system")),
    ];
    let selected = modes
        .iter()
        .position(|(mode, _)| mode == theme_mode)
        .unwrap_or(0);
    let labels: Vec<&str> = modes.iter().map(|(_, name)| *name).collect();
    if let Some(idx) = settings_ui::choice_list(ui, &labels, selected) {
        *theme_mode = modes[idx].0;
        changed = true;
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.theme_apply_immediately"))
            .size(12.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );

    ui.add_space(16.0);
    ui.label(t!("settings.interface_scale"));
    ui.horizontal_wrapped(|ui| {
        changed |= render_scale_slider(ui, interface_scale).1;
        if ui.button(t!("settings.interface_scale_reset")).clicked() {
            *interface_scale = crate::ui::interface_scale::DEFAULT;
            changed = true;
        }
    });
    ui.label(
        RichText::new(t!("settings.interface_scale_description"))
            .size(12.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );

    changed
}

fn render_scale_slider(ui: &mut egui::Ui, interface_scale: &mut f32) -> (egui::Response, bool) {
    let draft_id = ui.make_persistent_id("interface_scale_percent");
    let draft = ui.data(|data| data.get_temp::<f32>(draft_id));
    let mut percent = draft.unwrap_or(*interface_scale * 100.0);
    let previous = *interface_scale;
    let response = ui.add(
        egui::Slider::new(
            &mut percent,
            100.0..=crate::ui::interface_scale::MAX * 100.0,
        )
        .step_by(5.0)
        .suffix("%")
        .update_while_editing(false),
    );
    if response.is_pointer_button_down_on() {
        // Applying zoom here moves the centered settings window and its slider
        // under the held pointer, feeding the new position back into the value.
        ui.data_mut(|data| data.insert_temp(draft_id, percent));
    } else {
        ui.data_mut(|data| data.remove::<f32>(draft_id));
        if response.changed() || draft.is_some() {
            *interface_scale = percent / 100.0;
        }
    }
    (response, previous != *interface_scale)
}

#[cfg(test)]
mod tests;
