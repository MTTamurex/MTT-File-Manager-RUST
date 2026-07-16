use crate::app::global_search_state::GlobalSearchRenameState;
use eframe::egui;
use std::path::PathBuf;

pub(super) enum RenameInputAction {
    None,
    Cancel,
    Commit(PathBuf, String),
}

pub(super) fn render(
    ui: &mut egui::Ui,
    rename: &mut GlobalSearchRenameState,
    rect: egui::Rect,
    font: egui::FontId,
) -> RenameInputAction {
    let response = ui.put(
        rect,
        egui::TextEdit::singleline(&mut rename.text)
            .id_source(("global_search_rename", rename.source_index))
            .font(font)
            .margin(egui::Margin::symmetric(3, 1)),
    );
    if rename.focus_request {
        response.request_focus();
        select_editable_name(ui, &response, rename);
        rename.focus_request = false;
    }

    let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
    if enter_confirms_rename(enter_pressed, response.has_focus(), response.lost_focus()) {
        let new_name = rename.text.trim().to_string();
        if !new_name.is_empty() && new_name != rename.original_name {
            return RenameInputAction::Commit(PathBuf::from(&rename.path), new_name);
        }
        return RenameInputAction::Cancel;
    }
    if response.lost_focus() {
        return RenameInputAction::Cancel;
    }
    RenameInputAction::None
}

fn enter_confirms_rename(enter_pressed: bool, has_focus: bool, lost_focus: bool) -> bool {
    enter_pressed && (has_focus || lost_focus)
}

fn select_editable_name(
    ui: &egui::Ui,
    response: &egui::Response,
    rename: &GlobalSearchRenameState,
) {
    let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) else {
        return;
    };
    let char_count = rename.text.chars().count();
    let select_end = if rename.is_dir {
        char_count
    } else {
        rename
            .text
            .rfind('.')
            .map(|byte_pos| rename.text[..byte_pos].chars().count())
            .filter(|&position| position > 0)
            .unwrap_or(char_count)
    };
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(select_end),
        )));
    state.store(ui.ctx(), response.id);
}

#[cfg(test)]
mod tests {
    use super::enter_confirms_rename;

    #[test]
    fn enter_confirms_after_singleline_input_loses_focus() {
        assert!(enter_confirms_rename(true, false, true));
        assert!(enter_confirms_rename(true, true, false));
        assert!(!enter_confirms_rename(false, false, true));
    }
}
