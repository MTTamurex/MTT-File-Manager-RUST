use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

fn render_form(ui: &mut egui::Ui, name: &mut String, tab_count: usize) -> (bool, bool) {
    // A single-line TextEdit consumes Enter and relinquishes focus. Read it
    // before drawing the editor, not after.
    let enter_pressed =
        ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    ui.label(t!("tabs.save_group_count", count = tab_count));
    ui.add_space(8.0);
    ui.add(
        egui::TextEdit::singleline(name)
            .id(egui::Id::new("save_group_name"))
            .hint_text(t!("tabs.group_default_name"))
            .desired_width(ui.available_width()),
    );
    ui.add_space(8.0);
    let valid_name = !name.trim().is_empty();
    let mut save = valid_name && enter_pressed;
    let mut cancel = false;
    ui.horizontal(|ui| {
        save |= ui
            .add_enabled(valid_name, egui::Button::new(t!("tabs.save_group_save")))
            .clicked();
        cancel = ui.button(t!("tabs.save_group_cancel")).clicked();
    });
    (save, cancel)
}

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
            let (save, cancel) = render_form(ui, &mut app.save_group_name, tab_count);
            cancel_clicked = cancel;
            if save {
                submitted = Some(app.save_group_name.clone());
            }
        });

    if !open || cancel_clicked {
        app.show_save_group_dialog = false;
        app.save_group_name.clear();
    } else if let Some(name) = submitted {
        app.save_tabs_as_group(&name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_in_focused_name_editor_submits_only_a_nonempty_name() {
        for initial in ["Work", "   "] {
            let ctx = egui::Context::default();
            let mut name = initial.to_owned();
            let editor_id = egui::Id::new("save_group_name");
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                render_form(ui, &mut name, 3);
                ui.memory_mut(|memory| memory.request_focus(editor_id));
            });
            let mut submitted = false;
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: Some(egui::Key::Enter),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                assert!(ui.memory(|memory| memory.has_focus(editor_id)));
                let (save, cancel) = render_form(ui, &mut name, 3);
                submitted = save;
                assert!(!cancel);
            });
            assert_eq!(submitted, !initial.trim().is_empty());
        }
    }
}
