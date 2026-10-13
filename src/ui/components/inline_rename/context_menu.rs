use super::TextContextMenuResult;
use eframe::egui::{self, Id, Response, Ui};
use std::ops::Range;

#[derive(Clone)]
struct TextSelectionSnapshot {
    text: String,
    chars: Range<usize>,
}

fn text_selection_snapshot_id(edit_id: Id) -> Id {
    edit_id.with("inline_rename_selection_snapshot")
}

fn selected_char_range(cursor_range: Option<egui::text::CCursorRange>) -> Option<Range<usize>> {
    cursor_range
        .map(|range| range.as_sorted_char_range())
        .map(|range| range.start.0..range.end.0)
}

fn range_has_selection(range: &Range<usize>) -> bool {
    range.start < range.end
}

fn context_menu_selection(
    current: Option<Range<usize>>,
    saved: Option<Range<usize>>,
    preserve_saved: bool,
) -> Option<Range<usize>> {
    if preserve_saved {
        saved.or(current)
    } else {
        current
    }
}

fn matching_saved_selection(
    snapshot: Option<TextSelectionSnapshot>,
    text: &str,
) -> Option<Range<usize>> {
    snapshot
        .filter(|snapshot| snapshot.text == text)
        .map(|snapshot| snapshot.chars)
}

fn store_selection_snapshot(
    ctx: &egui::Context,
    id: Id,
    text: &str,
    selection: Option<Range<usize>>,
) {
    ctx.data_mut(|data| {
        if let Some(selection) = selection.filter(range_has_selection) {
            data.insert_temp(
                id,
                TextSelectionSnapshot {
                    text: text.to_owned(),
                    chars: selection,
                },
            );
        } else {
            data.remove::<TextSelectionSnapshot>(id);
        }
    });
}

fn restore_text_selection(ctx: &egui::Context, edit_id: Id, text: &str, selection: &Range<usize>) {
    let char_count = text.chars().count();
    let start = selection.start.min(char_count);
    let end = selection.end.min(char_count).max(start);
    let mut state = egui::TextEdit::load_state(ctx, edit_id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(start),
            egui::text::CCursor::new(end),
        )));
    state.store(ctx, edit_id);
}

/// Adds standard text-editing commands to an editable single-line text field.
pub(crate) fn add_text_context_menu(
    ui: &mut Ui,
    response: &Response,
    text: &mut String,
    reset_selection_snapshot: bool,
) -> TextContextMenuResult {
    #[derive(Clone, Copy)]
    enum Action {
        Cut,
        Copy,
        Paste,
        Delete,
        SelectAll,
    }

    let ctx = ui.ctx();
    let snapshot_id = text_selection_snapshot_id(response.id);
    if reset_selection_snapshot {
        store_selection_snapshot(ctx, snapshot_id, text, None);
    }
    let current_state = egui::TextEdit::load_state(ctx, response.id);
    let current_selection = (!reset_selection_snapshot).then(|| {
        selected_char_range(
            current_state
                .as_ref()
                .and_then(|state| state.cursor.char_range()),
        )
    });
    let current_selection = current_selection.flatten();
    let saved_selection = matching_saved_selection(
        ctx.data(|data| data.get_temp::<TextSelectionSnapshot>(snapshot_id)),
        text,
    );
    let secondary_clicked = response.secondary_clicked();
    let right_click_on_edit = secondary_clicked
        || ui.input(|input| {
            input.pointer.button_down(egui::PointerButton::Secondary) && response.hovered()
        });
    let menu_was_open = response.context_menu_opened();
    let selected_chars = context_menu_selection(
        current_selection.clone(),
        saved_selection.clone(),
        right_click_on_edit || menu_was_open,
    )
    .unwrap_or_else(|| {
        let end = text.chars().count();
        end..end
    });
    let has_selection = range_has_selection(&selected_chars);

    if right_click_on_edit || menu_was_open {
        if let Some(selection) = saved_selection
            .as_ref()
            .filter(|selection| range_has_selection(selection))
        {
            restore_text_selection(ctx, response.id, text, selection);
        } else if let Some(selection) = current_selection
            .as_ref()
            .filter(|selection| range_has_selection(selection))
        {
            store_selection_snapshot(ctx, snapshot_id, text, Some(selection.clone()));
        }
    } else {
        store_selection_snapshot(ctx, snapshot_id, text, current_selection.clone());
    }
    let mut action = None;

    let _menu_response = response.context_menu(|ui| {
        if ui
            .add_enabled(
                has_selection,
                egui::Button::new(rust_i18n::t!("context_menu.cut")),
            )
            .clicked()
        {
            action = Some(Action::Cut);
            ui.close();
        }
        if ui
            .add_enabled(
                has_selection,
                egui::Button::new(rust_i18n::t!("context_menu.copy")),
            )
            .clicked()
        {
            action = Some(Action::Copy);
            ui.close();
        }
        if ui.button(rust_i18n::t!("context_menu.paste")).clicked() {
            action = Some(Action::Paste);
            ui.close();
        }
        if ui
            .add_enabled(
                has_selection,
                egui::Button::new(rust_i18n::t!("context_menu.delete")),
            )
            .clicked()
        {
            action = Some(Action::Delete);
            ui.close();
        }
        ui.separator();
        if ui
            .button(rust_i18n::t!("settings.shortcut_select_all"))
            .clicked()
        {
            action = Some(Action::SelectAll);
            ui.close();
        }
    });

    let Some(action) = action else {
        return TextContextMenuResult {
            menu_active: response.context_menu_opened(),
            text_changed: false,
        };
    };

    let byte_range = char_range_to_byte_range(text, selected_chars.clone());
    let mut cursor_char = selected_chars.start.min(text.chars().count());
    let text_before_action = text.clone();
    match action {
        Action::Copy => {
            if has_selection {
                ui.ctx().copy_text(text[byte_range.clone()].to_owned());
            }
            response.request_focus();
            return TextContextMenuResult {
                menu_active: true,
                text_changed: false,
            };
        }
        Action::Cut => {
            if has_selection {
                ui.ctx().copy_text(text[byte_range.clone()].to_owned());
                text.replace_range(byte_range, "");
            }
        }
        Action::Paste => {
            let Some(pasted_text) = read_clipboard_text() else {
                response.request_focus();
                return TextContextMenuResult {
                    menu_active: true,
                    text_changed: false,
                };
            };
            let single_line_text = pasted_text.replace(['\r', '\n'], " ");
            cursor_char += single_line_text.chars().count();
            text.replace_range(byte_range, &single_line_text);
        }
        Action::Delete => {
            if has_selection {
                text.replace_range(byte_range, "");
            }
        }
        Action::SelectAll => {
            let state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
            let mut state = state;
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(text.chars().count()),
                )));
            state.store(ui.ctx(), response.id);
            store_selection_snapshot(ctx, snapshot_id, text, Some(0..text.chars().count()));
            response.request_focus();
            return TextContextMenuResult {
                menu_active: true,
                text_changed: false,
            };
        }
    }

    let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(cursor_char),
        )));
    state.store(ui.ctx(), response.id);
    store_selection_snapshot(ctx, snapshot_id, text, None);
    response.request_focus();
    TextContextMenuResult {
        menu_active: true,
        text_changed: *text != text_before_action,
    }
}

fn read_clipboard_text() -> Option<String> {
    use clipboard_win::{formats, Clipboard, Getter};

    let _clipboard = Clipboard::new_attempts(10).ok()?;
    let mut text = String::new();
    formats::Unicode.read_clipboard(&mut text).ok()?;
    Some(text)
}

fn char_range_to_byte_range(text: &str, range: Range<usize>) -> Range<usize> {
    fn char_to_byte(text: &str, char_index: usize) -> usize {
        text.char_indices()
            .nth(char_index)
            .map(|(byte_index, _)| byte_index)
            .unwrap_or(text.len())
    }

    let char_count = text.chars().count();
    let start = range.start.min(char_count);
    let end = range.end.min(char_count).max(start);
    char_to_byte(text, start)..char_to_byte(text, end)
}

#[cfg(test)]
mod tests {
    use super::{
        char_range_to_byte_range, context_menu_selection, matching_saved_selection,
        TextSelectionSnapshot,
    };

    #[test]
    fn char_ranges_map_to_utf8_boundaries() {
        let text = "aé👍b";
        assert_eq!(char_range_to_byte_range(text, 1..3), 1..7);
        assert_eq!(char_range_to_byte_range(text, 2..99), 3..8);
        assert_eq!(char_range_to_byte_range(text, 4..4), 8..8);
    }

    #[test]
    fn context_menu_keeps_saved_selection_after_right_click_moves_the_caret() {
        assert_eq!(
            context_menu_selection(Some(4..4), Some(1..4), true),
            Some(1..4)
        );
        assert_eq!(
            context_menu_selection(Some(4..4), Some(1..4), false),
            Some(4..4)
        );
    }

    #[test]
    fn stale_snapshot_is_ignored_when_the_text_edit_value_changes() {
        let snapshot = Some(TextSelectionSnapshot {
            text: "previous".to_owned(),
            chars: 2..6,
        });
        assert_eq!(matching_saved_selection(snapshot, "current"), None);
    }
}
