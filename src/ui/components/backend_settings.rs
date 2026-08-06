use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

const BACKENDS: &[(&str, &str)] = &[
    ("auto", "DirectX 12 (default)"),
    ("vulkan", "Vulkan"),
    ("glow", "OpenGL (fallback)"),
];

pub fn render_backend_settings_section(
    ui: &mut egui::Ui,
    active_backend: &str,
    gpu_backend_preference: &mut String,
) -> bool {
    let mut changed = false;
    let dark_mode = ui.visuals().dark_mode;

    settings_ui::section_header(
        ui,
        &t!("settings.graphics"),
        &t!("settings.graphics_description"),
    );

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
    ui.add_space(10.0);

    // Show currently active backend
    ui.horizontal(|ui| {
        ui.label(RichText::new(t!("settings.backend_active")).color(theme::text_color(dark_mode)));
        ui.label(
            RichText::new(active_backend)
                .strong()
                .color(ui.visuals().hyperlink_color),
        );
    });
    ui.add_space(12.0);

    // Backend selector
    ui.label(RichText::new(t!("settings.backend_select")).color(theme::text_color(dark_mode)));
    ui.add_space(6.0);

    let selected = BACKENDS
        .iter()
        .position(|&(value, _)| value == gpu_backend_preference.as_str())
        .unwrap_or(0);
    let labels: Vec<&str> = BACKENDS
        .iter()
        .map(|&(_, display_name)| display_name)
        .collect();
    if let Some(idx) = settings_ui::choice_list(ui, &labels, selected) {
        *gpu_backend_preference = BACKENDS[idx].0.to_string();
        changed = true;
    }

    ui.add_space(12.0);
    ui.label(
        RichText::new(t!("settings.backend_restart_warning"))
            .size(12.0)
            .color(ui.visuals().warn_fg_color),
    );

    changed
}
