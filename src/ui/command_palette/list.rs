use super::{input_id, keyboard::Keys, row, selection, tooltip};
use crate::app::command_palette::{
    availability::{disabled_reason, CommandContext},
    catalog::Command,
    search, CommandPaletteState,
};
use crate::app::shortcuts::ShortcutBindings;
use eframe::egui;
use rust_i18n::t;

pub(super) fn render(
    ui: &mut egui::Ui,
    state: &mut CommandPaletteState,
    context: &CommandContext,
    shortcuts: &ShortcutBindings,
    keys: &Keys,
    pointer_moved: bool,
) -> Option<Command> {
    let mut execute = None;
    ui.heading(t!("command_palette.title"));
    ui.add_space(ui.text_style_height(&egui::TextStyle::Body));
    let input = ui.add(
        egui::TextEdit::singleline(&mut state.query)
            .id(input_id())
            .hint_text(t!("command_palette.placeholder"))
            .desired_width(f32::INFINITY),
    );
    if state.request_focus {
        input.request_focus();
        state.request_focus = false;
    }
    if input.changed() {
        state.selected = 0;
        state.reveal_selected = true;
    }
    let entries = search::entries(&state.query, shortcuts);
    state.selected = state.selected.min(entries.len().saturating_sub(1));
    if keys.up {
        state.move_selection(entries.len(), true);
    }
    if keys.down {
        state.move_selection(entries.len(), false);
    }
    ui.separator();
    let mouse_selects = pointer_moved && !keys.up && !keys.down && !keys.run && !input.changed();
    let footer = t!("command_palette.hint").to_string();
    let footer_height = ui
        .painter()
        .layout(
            footer.clone(),
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
            ui.available_width(),
        )
        .size()
        .y;
    let results_height =
        (ui.available_height() - footer_height - ui.spacing().item_spacing.y * 3.0 - 6.0).max(0.0);
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.floating_allocated_width = scroll.bar_width + 8.0;
    egui::ScrollArea::vertical()
        .id_salt("command_palette_results")
        .max_height(results_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if entries.is_empty() {
                ui.label(t!("command_palette.no_matches"));
            }
            for (index, entry) in entries.iter().enumerate() {
                let reason = disabled_reason(entry.command, context);
                let selected = index == state.selected;
                let row = row::render(ui, entry, selected, reason.is_none());
                if mouse_selects && row.contains_pointer() && !row.clicked() {
                    selection::select_hovered(state, index, ui.ctx());
                }
                if selected && state.reveal_selected {
                    row.scroll_to_me(Some(egui::Align::Center));
                }
                if let Some(reason) = reason.filter(|_| index == state.selected) {
                    let _ = tooltip::show(&row, &t!(reason));
                }
                if row.clicked() {
                    state.selected = index;
                    execute = Some(entry.command);
                }
            }
        });
    state.reveal_selected = false;
    if keys.run {
        if let Some(entry) = entries.get(state.selected) {
            if disabled_reason(entry.command, context).is_none() {
                execute = Some(entry.command);
            }
        }
    }
    ui.separator();
    ui.add(egui::Label::new(footer).wrap());
    execute
}
