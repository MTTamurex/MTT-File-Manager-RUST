use eframe::egui;
use rust_i18n::t;

const BACKENDS: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("dx12", "DirectX 12"),
    ("vulkan", "Vulkan"),
    ("gl", "OpenGL"),
];

const RENDERERS: &[(&str, &str)] = &[
    ("wgpu", "Wgpu (DX12 / Vulkan / OpenGL)"),
    ("glow", "Glow (OpenGL — menor uso de RAM)"),
];

pub fn render_backend_settings_section(
    ui: &mut egui::Ui,
    active_backend: &str,
    gpu_backend_preference: &mut String,
    renderer_preference: &mut String,
) -> bool {
    let mut changed = false;

    ui.label(egui::RichText::new(t!("settings.backend_title").to_string()).strong());
    ui.add_space(4.0);
    ui.label(t!("settings.backend_description"));
    ui.add_space(12.0);

    // Renderer selector (Wgpu vs Glow). This is the first choice because it
    // determines whether the GPU backend selector below has any effect: Glow
    // ignores the wgpu backend list entirely.
    ui.label(egui::RichText::new(t!("settings.renderer_title").to_string()).strong());
    ui.add_space(4.0);
    ui.label(t!("settings.renderer_description"));
    ui.add_space(8.0);

    for &(value, display_name) in RENDERERS {
        let is_selected = *renderer_preference == value;
        if ui.selectable_label(is_selected, display_name).clicked() && !is_selected {
            *renderer_preference = value.to_string();
            changed = true;
        }
    }

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(12.0);

    // Show currently active backend
    ui.horizontal(|ui| {
        ui.label(t!("settings.backend_active"));
        ui.label(
            egui::RichText::new(active_backend)
                .strong()
                .color(ui.visuals().hyperlink_color),
        );
    });
    ui.add_space(12.0);

    // Backend selector — only meaningful when the Wgpu renderer is active.
    let backend_selector_enabled = renderer_preference != "glow";
    ui.label(t!("settings.backend_select"));
    ui.add_space(4.0);

    ui.add_enabled_ui(backend_selector_enabled, |ui| {
        for &(value, display_name) in BACKENDS {
            let is_selected = *gpu_backend_preference == value;
            if ui.selectable_label(is_selected, display_name).clicked() && !is_selected {
                *gpu_backend_preference = value.to_string();
                changed = true;
            }
        }
    });

    if !backend_selector_enabled {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(t!("settings.backend_glow_hint").to_string())
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }

    ui.add_space(12.0);
    ui.label(
        egui::RichText::new(t!("settings.backend_restart_warning").to_string())
            .small()
            .color(ui.visuals().warn_fg_color),
    );
    ui.add_space(8.0);
    ui.separator();

    changed
}
