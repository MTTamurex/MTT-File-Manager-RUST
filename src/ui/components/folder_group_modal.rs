use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

pub(crate) fn render_save_group_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if !app.show_save_group_dialog {
        return;
    }

    let tab_count = app.workspace_tab_paths().len();
    let mut open = true;
    let mut cancel_clicked = false;
    let mut submitted = None;
    egui::Window::new(t!("tabs.save_group_title"))
        .id(egui::Id::new("save_tabs_group_window"))
        .collapsible(false)
        .resizable(false)
        .default_width(360.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .order(egui::Order::Foreground)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(t!("tabs.save_group_count", count = tab_count));
            ui.add_space(8.0);
            let response = ui.add(
                egui::TextEdit::singleline(&mut app.save_group_name)
                    .hint_text(t!("tabs.group_default_name"))
                    .desired_width(ui.available_width()),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !app.save_group_name.trim().is_empty(),
                        egui::Button::new(t!("tabs.save_group_save")),
                    )
                    .clicked()
                    || (response.has_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                {
                    submitted = Some(app.save_group_name.clone());
                }
                if ui.button(t!("tabs.save_group_cancel")).clicked() {
                    cancel_clicked = true;
                }
            });
        });

    if let Some(name) = submitted {
        app.save_tabs_as_group(&name);
    }
    if !open || cancel_clicked {
        app.show_save_group_dialog = false;
        app.save_group_name.clear();
    }
}
