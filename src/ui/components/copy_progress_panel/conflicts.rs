//! Conflict controls never probe the filesystem on the UI thread.
use super::path_row;
use crate::infrastructure::copy_engine::CopyOperation;
use eframe::egui::{self, RichText};
use mtt_copy_engine::conflicts::{Conflict, ConflictAction, ConflictKind};
use rust_i18n::t;

pub(super) fn render(ui: &mut egui::Ui, operation: &CopyOperation, request: &Conflict) {
    let progress = &operation.progress;
    let question = match request.kind {
        ConflictKind::File => t!("copy_engine.file_conflict"),
        ConflictKind::Directory => t!("copy_engine.folder_conflict"),
        ConflictKind::TypeMismatch => t!("copy_engine.type_conflict"),
        ConflictKind::SamePath => t!("copy_engine.self_conflict"),
        ConflictKind::Queued => t!("copy_engine.queued_conflict"),
    };
    ui.label(RichText::new(question).strong());
    let name = request
        .destination
        .file_name()
        .unwrap_or(request.destination.as_os_str())
        .to_string_lossy();
    ui.add(egui::Label::new(RichText::new(name).strong()).truncate())
        .on_hover_text(super::display_paths::text(
            &request.destination.display().to_string(),
        ));
    ui.add_space(12.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        path_row(
            ui,
            &t!("copy_engine.source"),
            &request.source.display().to_string(),
            || request.source.display().to_string(),
        );
        path_row(
            ui,
            &t!("copy_engine.destination"),
            &request.destination.display().to_string(),
            || request.destination.display().to_string(),
        );
    });
    ui.add_space(12.0);
    let scope_id = ui.make_persistent_id("copy_conflict_scope");
    let mut all = ui
        .data(|data| data.get_temp::<(u64, bool)>(scope_id))
        .filter(|(id, _)| *id == request.id)
        .is_some_and(|(_, value)| value);
    let checkbox = ui.checkbox(&mut all, t!("copy_engine.apply_conflicts"));
    if checkbox.clicked() {
        checkbox.request_focus();
    }
    let mut focused = checkbox.has_focus() || checkbox.lost_focus();
    ui.data_mut(|data| data.insert_temp(scope_id, (request.id, all)));
    ui.add_space(12.0);
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        let cancel = ui.button(t!("copy_engine.cancel"));
        let focus_id = ui.make_persistent_id("copy_conflict_focus");
        if ui.data(|data| data.get_temp::<u64>(focus_id)) != Some(request.id) {
            cancel.request_focus();
            ui.data_mut(|data| data.insert_temp(focus_id, request.id));
        }
        focused |= cancel.has_focus() || cancel.lost_focus();
        if cancel.clicked() {
            action = Some(ConflictAction::Cancel);
        }
        for (choice, label) in [
            (ConflictAction::Skip, t!("copy_engine.skip")),
            (ConflictAction::KeepBoth, t!("copy_engine.keep_both")),
            (ConflictAction::Merge, t!("copy_engine.merge")),
            (ConflictAction::Replace, t!("copy_engine.replace")),
        ] {
            if !request.allows(choice) {
                continue;
            }
            let color = if choice == ConflictAction::Replace {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().text_color()
            };
            let button = ui.add_enabled(
                !progress.is_cancelled(),
                egui::Button::new(RichText::new(label).color(color)),
            );
            focused |= button.has_focus() || button.lost_focus();
            if button.clicked() {
                action = Some(choice);
            }
        }
    });
    if focused && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        action = Some(ConflictAction::Cancel);
    }
    if let Some(action) = action {
        if action == ConflictAction::Cancel {
            progress.cancel();
        }
        progress.conflicts.respond(request.id, action, all);
    }
}

#[cfg(test)]
mod tests;
