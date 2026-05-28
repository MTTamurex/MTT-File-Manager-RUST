use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

const BACKENDS: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("dx12", "DirectX 12"),
    ("vulkan", "Vulkan"),
    ("gl", "OpenGL"),
];

pub fn render_backend_settings_section(
    ui: &mut egui::Ui,
    active_backend: &str,
    gpu_backend_preference: &mut String,
) -> bool {
    let mut changed = false;
    let dark_mode = ui.visuals().dark_mode;

    ui.label(
        RichText::new(t!("settings.backend_title"))
            .size(14.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.backend_description"))
            .size(13.0)
            .color(theme::secondary_text_color(dark_mode)),
    );
    ui.add_space(12.0);

    // Show currently active backend
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(t!("settings.backend_active")).color(theme::text_color(dark_mode)),
        );
        ui.label(
            RichText::new(active_backend)
                .strong()
                .color(ui.visuals().hyperlink_color),
        );
    });
    ui.add_space(12.0);

    // Backend selector
    ui.label(
        RichText::new(t!("settings.backend_select")).color(theme::text_color(dark_mode)),
    );
    ui.add_space(4.0);

    for &(value, display_name) in BACKENDS {
        let is_selected = *gpu_backend_preference == value;
        if ui.selectable_label(is_selected, display_name).clicked() && !is_selected {
            *gpu_backend_preference = value.to_string();
            changed = true;
        }
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.backend_restart_warning"))
            .size(12.0)
            .color(ui.visuals().warn_fg_color),
    );
    ui.add_space(8.0);
    ui.separator();

    changed
}
