//! Confirmation is part of the operation card, never a native modal dialog.
use crate::infrastructure::copy_engine::CopyOperation;
use crate::infrastructure::delete_engine::confirmation::{Kind, Request};
use eframe::egui::{self, RichText};
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, operation: &CopyOperation, request: &Request) {
    let deletion = operation.deletion.as_ref().unwrap();
    let permanent = request.kind != Kind::Recycle;
    let message = match request.kind {
        Kind::Permanent => t!(
            "operations.search_delete_permanent_confirm",
            count = request.paths.len()
        ),
        Kind::Recycle => t!("delete_engine.recycle_confirm", count = request.paths.len()),
        Kind::RecycleUnavailable => t!("delete_engine.fallback_confirm"),
    };
    ui.add_space(8.0);
    let color = if permanent {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().text_color()
    };
    ui.label(RichText::new(message).strong().color(color));
    egui::ScrollArea::vertical()
        .id_salt("delete_confirmation_paths")
        .max_height(96.0)
        .show_rows(
            ui,
            ui.text_style_height(&egui::TextStyle::Body),
            request.paths.len(),
            |ui, range| {
                for index in range {
                    let path = request.paths[index].display().to_string();
                    ui.add(egui::Label::new(&path).truncate())
                        .on_hover_text(path);
                }
            },
        );
    ui.horizontal_wrapped(|ui| {
        let cancel = ui.button(t!("copy_engine.cancel"));
        let focus_id = ui.make_persistent_id("delete_confirmation_focus");
        if ui.data(|data| data.get_temp::<u64>(focus_id)) != Some(request.id) {
            cancel.request_focus();
            ui.data_mut(|data| data.insert_temp(focus_id, request.id));
        }
        let label = if permanent {
            t!("delete_engine.confirm_permanent")
        } else {
            t!("delete_engine.confirm_recycle")
        };
        let confirm = ui.add_enabled(
            !operation.progress.is_cancelled(),
            egui::Button::new(RichText::new(label).color(color)),
        );
        // egui may surrender keyboard focus at frame start when Esc is pressed.
        let escape = (cancel.has_focus()
            || confirm.has_focus()
            || cancel.lost_focus()
            || confirm.lost_focus())
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel.clicked() || escape {
            operation.progress.cancel();
            deletion.confirmation.respond(request.id, false);
        } else if confirm.clicked() {
            deletion.confirmation.respond(request.id, true);
        }
    });
}
