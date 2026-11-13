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
    ui.add_space(4.0);
    let color = if permanent {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().text_color()
    };
    ui.label(RichText::new(message).strong().color(color));
    ui.add_space(12.0);
    // Text galleys round to whole points; reserve that same height so the
    // fifth row is not fractionally clipped at either interface scale.
    let row_height = ui.text_style_height(&egui::TextStyle::Body).ceil();
    let list_height = 5.0 * row_height + 4.0 * ui.spacing().item_spacing.y;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("delete_confirmation_paths")
            .max_height(list_height)
            .min_scrolled_height(row_height)
            .auto_shrink([false, true])
            .show_rows(ui, row_height, request.paths.len(), |ui, range| {
                for index in range {
                    let path = &request.paths[index];
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    ui.add(egui::Label::new(name).truncate().halign(egui::Align::Min))
                        .on_hover_text(super::super::display_paths::text(
                            &path.display().to_string(),
                        ));
                }
            });
    });
    ui.add_space(12.0);
    let mut apply_to_remaining = false;
    let mut scope_had_focus = false;
    if request.kind == Kind::RecycleUnavailable {
        let scope_id = ui.make_persistent_id("delete_confirmation_scope");
        apply_to_remaining = ui
            .data(|data| data.get_temp::<(u64, bool)>(scope_id))
            .filter(|(id, _)| *id == request.id)
            .is_some_and(|(_, value)| value);
        let checkbox = ui.checkbox(
            &mut apply_to_remaining,
            t!("delete_engine.apply_to_remaining"),
        );
        if checkbox.clicked() {
            checkbox.request_focus();
        }
        scope_had_focus = checkbox.has_focus() || checkbox.lost_focus();
        ui.data_mut(|data| data.insert_temp(scope_id, (request.id, apply_to_remaining)));
        ui.add_space(12.0);
    }
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
            || confirm.lost_focus()
            || scope_had_focus)
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if cancel.clicked() || escape {
            operation.progress.cancel();
            deletion.confirmation.respond(request.id, false);
        } else if confirm.clicked() {
            deletion
                .confirmation
                .respond_with_scope(request.id, true, apply_to_remaining);
        }
    });
    ui.add_space(4.0);
}
