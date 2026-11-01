use crate::application::ClipboardOp;
use crate::ui::{clipboard_feedback, components::settings_ui};
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, app: &mut crate::app::ImageViewerApp) {
    settings_ui::subsection_header(ui, &t!("settings.clipboard_highlights"), "");
    let preferences = &mut app.clipboard_highlights;
    let previous = *preferences;
    settings_ui::toggle_row(
        ui,
        &t!("settings.highlight_copied_cut_items"),
        &mut preferences.enabled,
    );
    ui.add_enabled_ui(preferences.enabled, |ui| {
        for (operation, label) in [
            (ClipboardOp::Copy, t!("settings.copied_items_color")),
            (ClipboardOp::Move, t!("settings.cut_items_color")),
        ] {
            let color = preferences.color(operation, ui.visuals().dark_mode);
            let mut rgb = [color.r(), color.g(), color.b()];
            ui.horizontal(|ui| {
                ui.label(label.as_ref());
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    match operation {
                        ClipboardOp::Copy => preferences.copy_color = Some(rgb),
                        ClipboardOp::Move => preferences.cut_color = Some(rgb),
                    }
                }
            });
        }
    });
    if previous != *preferences {
        clipboard_feedback::remember(ui.ctx(), *preferences);
        app.save_preferences();
        app.save_preferences_async();
    }
}
