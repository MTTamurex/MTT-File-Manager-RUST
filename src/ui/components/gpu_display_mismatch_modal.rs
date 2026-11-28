//! Warns that the GPU chosen in Settings does not drive the monitor, so every
//! frame is copied between GPUs (see `gpu_backend::display_guard`).

use crate::app::state::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

const MAX_WIDTH: f32 = 520.0;

pub fn render(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(notice) = app.gpu_display_mismatch_notice.as_ref() else {
        return;
    };
    let mut use_automatic = false;
    let mut dismissed = false;
    let response = egui::Modal::new(egui::Id::new("gpu_display_mismatch_notice"))
        .frame(egui::Frame::window(&ctx.global_style()).inner_margin(16.0))
        .show(ctx, |ui| {
            ui.set_max_width(MAX_WIDTH.min((ctx.content_rect().width() - 48.0).max(1.0)));
            ui.heading(t!("settings.gpu_display_mismatch_title"));
            ui.add_space(8.0);
            ui.label(t!(
                "settings.gpu_display_mismatch_body",
                preferred = notice.preferred.as_str(),
                display = notice.display.as_str()
            ));
            ui.add_space(6.0);
            ui.label(t!(
                "settings.gpu_display_mismatch_hint",
                display = notice.display.as_str()
            ));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                use_automatic = ui
                    .button(t!("settings.gpu_display_mismatch_use_automatic"))
                    .clicked();
                dismissed = ui.button(t!("settings.gpu_display_mismatch_ok")).clicked();
            });
        });
    if use_automatic {
        app.gpu_adapter_preference = crate::gpu_backend::GpuPreference::Automatic
            .key()
            .to_string();
        app.save_preferences();
    }
    if use_automatic || dismissed || response.should_close() {
        app.gpu_display_mismatch_notice = None;
    }
}
