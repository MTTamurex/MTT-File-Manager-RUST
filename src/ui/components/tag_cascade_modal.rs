use crate::app::ImageViewerApp;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

/// Confirms removing a parent tag's sub-tags from the selected files. Shown
/// only when unchecking would remove at least one sub-tag assignment.
pub fn render_tag_cascade_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some((tag_name, count)) = app
        .pending_tag_cascade
        .as_ref()
        .map(|pending| (pending.tag_name.clone(), pending.sub_tag_assignments))
    else {
        return;
    };

    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.cancel_pending_tag_cascade();
        return;
    }

    let mut confirm = false;
    let mut cancel = false;
    egui::Window::new(t!("tags.cascade_title"))
        .id(egui::Id::new("tag_cascade_confirm"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(
                    t!(
                        "tags.cascade_message",
                        name = tag_name.clone(),
                        count = count
                    )
                    .to_string(),
                )
                .color(ui.visuals().warn_fg_color),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(t!("tags.cascade_files_safe").to_string())
                    .color(theme::secondary_text_color(ui.ctx())),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(t!("tags.cascade_confirm")).clicked() {
                    confirm = true;
                }
                if ui.button(t!("tags.cascade_cancel")).clicked() {
                    cancel = true;
                }
            });
        });

    if confirm {
        app.confirm_pending_tag_cascade();
    } else if cancel {
        app.cancel_pending_tag_cascade();
    }
}
