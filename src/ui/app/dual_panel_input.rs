use eframe::egui;

pub(super) fn consume_panel_tab(ctx: &egui::Context, enabled: bool) -> bool {
    if !enabled
        || !ctx.input(|input| input.viewport().focused.unwrap_or(false))
        || ctx.text_edit_focused()
        || egui::Popup::is_any_open(ctx)
        || ctx.memory(|memory| memory.top_modal_layer().is_some())
    {
        return false;
    }

    let consumed = ctx.input_mut(|input| {
        let mut consumed = false;
        input.events.retain(|event| {
            let matches = matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.any()
            );
            consumed |= matches;
            !matches
        });
        consumed
    });
    if consumed {
        ctx.memory_mut(|memory| {
            // egui schedules Tab traversal at begin_pass, before event consumption.
            // Cancel it as well, or rendering the next widget steals pane focus.
            memory.move_focus(egui::FocusDirection::None);
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }
    consumed
}

#[cfg(test)]
mod tests;
