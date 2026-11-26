use crate::ui::components::mpv::video_gpu::VideoGpuPreference;
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

fn label_key(preference: VideoGpuPreference) -> &'static str {
    match preference {
        VideoGpuPreference::FollowApp => "settings.video_gpu_follow_app",
        VideoGpuPreference::HighPerformance => "settings.gpu_high_performance",
        VideoGpuPreference::LowPower => "settings.gpu_low_power",
        VideoGpuPreference::Automatic => "settings.gpu_automatic",
    }
}

/// GPU that mpv decodes and renders video on. Disabled with a single GPU,
/// where every option resolves to the same adapter. Returns true on change.
pub fn render_video_gpu_section(
    ui: &mut egui::Ui,
    preference: &mut VideoGpuPreference,
    single_adapter: bool,
) -> bool {
    ui.add_space(16.0);
    ui.label(
        RichText::new(t!("settings.video_gpu_title"))
            .size(14.0)
            .strong()
            .color(theme::text_color(ui.ctx())),
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(t!("settings.video_gpu_description"))
            .size(13.0)
            .color(theme::secondary_text_color(ui.ctx())),
    );
    ui.add_space(10.0);

    let labels: Vec<String> = VideoGpuPreference::ALL
        .iter()
        .map(|&option| t!(label_key(option)).to_string())
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let selected = VideoGpuPreference::ALL
        .iter()
        .position(|&option| option == *preference)
        .unwrap_or_default();

    let mut changed = false;
    ui.add_enabled_ui(!single_adapter, |ui| {
        if let Some(idx) = settings_ui::choice_list(ui, &label_refs, selected) {
            *preference = VideoGpuPreference::ALL[idx];
            changed = true;
        }
    });
    changed
}
