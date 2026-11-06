use super::model::Tab;
use crate::{
    app::ImageViewerApp,
    ui::components::{auto_tag_settings, tag_manager_modal},
};
use eframe::egui;

pub(super) fn show(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    host: egui::Id,
    creation: &mut Option<Tab>,
) {
    let Some(tab) = *creation else {
        return;
    };
    let mut cancelled = false;
    let mut created = false;
    let response = egui::Modal::new(host.with("create_item"))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .inner_margin(24.0)
                .corner_radius(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width((ctx.viewport_rect().width() - 80.0).clamp(180.0, 440.0));
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
            egui::ScrollArea::vertical()
                .id_salt("creation_body")
                .max_height((ctx.viewport_rect().height() - 112.0).max(80.0))
                .show(ui, |ui| {
                    created = match tab {
                        Tab::Tags => {
                            tag_manager_modal::render_tag_create_form(app, ui, &mut cancelled)
                        }
                        Tab::AutoTag => {
                            auto_tag_settings::render_add_rule_form(app, ui, &mut cancelled)
                        }
                    }
                    .is_some();
                });
        });
    if created || cancelled || response.should_close() {
        // Closing creation never changes the browser's selection, query or
        // edit drafts. The existing editor remains underneath the modal.
        *creation = None;
    }
}
