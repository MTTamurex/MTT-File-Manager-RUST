use crate::app::ImageViewerApp;
use crate::infrastructure::windows::app_instances::{
    allows_multiple_instances, ALLOW_MULTIPLE_INSTANCES_KEY,
};
use crate::ui::{components::settings_ui, theme};
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, app: &mut ImageViewerApp) {
    // Read the shared preference rather than caching it per instance. It is
    // written independently of session snapshots so older instances cannot
    // overwrite the policy when saving their own preferences.
    let value = app
        .app_state_db
        .get_preference(ALLOW_MULTIPLE_INSTANCES_KEY);
    let mut allow_multiple = allows_multiple_instances(value.as_deref());
    if settings_ui::toggle_row(
        ui,
        &t!("settings.allow_multiple_instances"),
        &mut allow_multiple,
    ) {
        match app.app_state_db.set_preference(
            ALLOW_MULTIPLE_INSTANCES_KEY,
            if allow_multiple { "true" } else { "false" },
        ) {
            Ok(()) if !allow_multiple => {
                app.notifications
                    .info(t!("settings.existing_instances_remain_open").to_string());
            }
            Ok(()) => {}
            Err(error) => {
                log::warn!("[INSTANCES] Could not save the instance preference: {error}");
                app.notifications
                    .error(t!("settings.instance_preference_save_failed").to_string());
                allow_multiple = !allow_multiple;
            }
        }
    }
    if !allow_multiple {
        ui.label(
            RichText::new(t!("settings.existing_instances_remain_open"))
                .size(12.0)
                .color(theme::secondary_text_color(ui.ctx())),
        );
    }
}
