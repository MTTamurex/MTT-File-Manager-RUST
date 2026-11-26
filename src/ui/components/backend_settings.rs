use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

const BACKENDS: &[(&str, &str)] = &[
    ("auto", "DirectX 12 (default)"),
    ("vulkan", "Vulkan"),
    ("glow", "OpenGL (fallback)"),
];

const GPU_PREFERENCES: &[(&str, &str)] = &[
    ("high_performance", "settings.gpu_high_performance"),
    ("low_power", "settings.gpu_low_power"),
    ("automatic", "settings.gpu_automatic"),
];

/// Read-only facts about the renderer this process is running with.
pub struct ActiveGraphics<'a> {
    pub backend: &'a str,
    pub device: &'a str,
    /// GPU driving the monitor, when it is not the rendering GPU.
    pub cross_adapter_display: Option<&'a str>,
    /// Only one hardware GPU exists, so the GPU preference cannot change anything.
    pub single_adapter: bool,
}

pub fn render_backend_settings_section(
    ui: &mut egui::Ui,
    active: ActiveGraphics<'_>,
    gpu_backend_preference: &mut String,
    gpu_adapter_preference: &mut String,
    scroll_interpolation_enabled: &mut bool,
) -> (bool, bool) {
    let ActiveGraphics {
        backend: active_backend,
        device: active_gpu_device,
        cross_adapter_display,
        single_adapter,
    } = active;
    let mut backend_changed = false;
    let mut interpolation_changed = false;

    settings_ui::section_header(
        ui,
        &t!("settings.graphics"),
        &t!("settings.graphics_description"),
    );

    ui.label(
        RichText::new(t!("settings.backend_title"))
            .size(14.0)
            .strong()
            .color(theme::text_color(ui.ctx())),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.backend_description"))
            .size(13.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );
    ui.add_space(10.0);
    let selected = BACKENDS
        .iter()
        .position(|&(value, _)| value == gpu_backend_preference.as_str())
        .unwrap_or(0);
    let labels: Vec<&str> = BACKENDS
        .iter()
        .map(|&(_, display_name)| display_name)
        .collect();
    let vulkan_limitations = t!("settings.vulkan_limitations_tooltip");
    let warnings: Vec<Option<&str>> = BACKENDS
        .iter()
        .map(|&(value, _)| (value == "vulkan").then_some(vulkan_limitations.as_ref()))
        .collect();
    if let Some(idx) = settings_ui::choice_list_with_warnings(ui, &labels, selected, &warnings) {
        *gpu_backend_preference = BACKENDS[idx].0.to_string();
        backend_changed = true;
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(t!("settings.backend_active")).color(theme::text_color(ui.ctx())));
        let active_backend_label = if active_backend.eq_ignore_ascii_case("dx12") {
            "DirectX 12"
        } else if active_backend.eq_ignore_ascii_case("glow") {
            "OpenGL"
        } else {
            active_backend
        };
        ui.label(
            RichText::new(active_backend_label)
                .strong()
                .color(ui.visuals().hyperlink_color),
        );
    });

    ui.add_space(16.0);
    ui.label(
        RichText::new(t!("settings.gpu_preference_title"))
            .size(14.0)
            .strong()
            .color(theme::text_color(ui.ctx())),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.gpu_preference_description"))
            .size(13.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new(t!("settings.gpu_glow_note"))
            .size(12.0)
            .color(ui.visuals().warn_fg_color),
    );
    ui.add_space(14.0);
    let selected_gpu = GPU_PREFERENCES
        .iter()
        .position(|&(value, _)| value == gpu_adapter_preference.as_str())
        .unwrap_or(0);
    let gpu_labels: Vec<String> = GPU_PREFERENCES
        .iter()
        .map(|&(_, key)| t!(key).to_string())
        .collect();
    let gpu_label_refs: Vec<&str> = gpu_labels.iter().map(String::as_str).collect();
    ui.add_enabled_ui(gpu_backend_preference != "glow" && !single_adapter, |ui| {
        if let Some(idx) = settings_ui::choice_list(ui, &gpu_label_refs, selected_gpu) {
            *gpu_adapter_preference = GPU_PREFERENCES[idx].0.to_string();
            backend_changed = true;
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(t!("settings.gpu_active")).color(theme::text_color(ui.ctx())));
        ui.label(
            RichText::new(if active_gpu_device.is_empty() {
                t!("settings.gpu_active_unknown").to_string()
            } else {
                active_gpu_device.to_string()
            })
            .strong()
            .color(ui.visuals().hyperlink_color),
        );
    });
    if let Some(display) = cross_adapter_display {
        ui.add_space(6.0);
        ui.label(
            RichText::new(t!("settings.gpu_cross_adapter_warning", display = display))
                .size(12.0)
                .color(ui.visuals().warn_fg_color),
        );
    }

    ui.add_space(16.0);
    ui.label(
        RichText::new(t!("settings.graphics_interpolation"))
            .size(14.0)
            .strong()
            .color(theme::text_color(ui.ctx())),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.graphics_interpolation_description"))
            .size(13.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );
    ui.add_space(6.0);

    if settings_ui::toggle_row(
        ui,
        &t!("settings.graphics_interpolation_toggle"),
        scroll_interpolation_enabled,
    ) {
        interpolation_changed = true;
    }

    ui.add_space(6.0);
    ui.label(
        RichText::new(t!("settings.graphics_interpolation_opengl_warning"))
            .size(12.0)
            .color(ui.visuals().warn_fg_color),
    );

    (backend_changed, interpolation_changed)
}
