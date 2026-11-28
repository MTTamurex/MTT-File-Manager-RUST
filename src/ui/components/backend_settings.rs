use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

/// `None` labels are translated: the automatic renderer is DirectX 12, except
/// on an Intel GPU where it is Vulkan (see `gpu_backend::intel_vulkan`).
const BACKENDS: &[(&str, Option<&str>)] = &[
    ("auto", None),
    ("vulkan", Some("Vulkan")),
    ("glow", Some("OpenGL (fallback)")),
];

/// Read-only facts about the renderer this process is running with.
pub struct ActiveGraphics<'a> {
    pub backend: &'a str,
    pub device: &'a str,
}

pub fn render_backend_settings_section(
    ui: &mut egui::Ui,
    active: ActiveGraphics<'_>,
    gpu_backend_preference: &mut String,
    scroll_interpolation_enabled: &mut bool,
) -> (bool, bool) {
    let ActiveGraphics {
        backend: active_backend,
        device: active_gpu_device,
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
    let labels: Vec<String> = BACKENDS
        .iter()
        .map(|&(_, display_name)| {
            display_name.map_or_else(
                || t!("settings.backend_automatic").to_string(),
                str::to_owned,
            )
        })
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    if let Some(idx) = settings_ui::choice_list(ui, &label_refs, selected) {
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
