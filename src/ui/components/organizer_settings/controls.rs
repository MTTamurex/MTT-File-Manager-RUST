use crate::app::ImageViewerApp;
use crate::ui::components::settings_ui;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render_global_toggle(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    settings_ui::row_frame(ui.ctx()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let state = &mut app.organizer_state;
        ui.horizontal(|ui| {
            let label = t!("organizer.enable_disable_all");
            ui.label(label.as_ref())
                .on_hover_text(t!("organizer.global_toggle_hint"));
            if state.global_pause.is_pending() {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut enabled = !state.global_pause.paused;
                let response = ui
                    .add_enabled_ui(!state.global_pause.is_pending(), |ui| {
                        settings_ui::toggle_switch(ui, &mut enabled, label.as_ref())
                    })
                    .inner;
                if response.clicked() {
                    if let Err(error) = state.global_pause.toggle(&state.manager) {
                        app.notifications.warning(error.to_string());
                    }
                }
            });
        });
    });
    ui.add_space(12.0);
}
