//! File shortcuts apply only to the explicit, visible search selection.
use super::actions::ResultAction;
use crate::app::global_search_state::GlobalSearchState;
use crate::app::shortcuts::{ShortcutAction, ShortcutBindings};
use eframe::egui;
use std::path::PathBuf;

pub(super) fn selection_action(
    state: &mut GlobalSearchState,
    shortcuts: &ShortcutBindings,
    ctx: &egui::Context,
    ordered_indices: &[usize],
) -> Option<ResultAction> {
    // Filter text fields can retain focus even when the result selection is active.
    if ctx.egui_wants_keyboard_input() {
        return None;
    }
    if shortcuts.is_triggered(ShortcutAction::SelectAll, ctx) {
        state.selected_indices.clear();
        state
            .selected_indices
            .extend(ordered_indices.iter().copied());
        state.selected_index = ordered_indices.first().copied();
        state.selection_anchor = state.selected_index;
        return None;
    }
    let paths = || {
        ordered_indices
            .iter()
            .filter(|idx| state.selected_indices.contains(idx))
            .filter_map(|idx| state.results.get(*idx))
            .map(|result| PathBuf::from(&result.full_path))
            .collect::<Vec<_>>()
    };
    // Permanent delete must take precedence, including customized shortcut bindings.
    if shortcuts.consume_triggered(ShortcutAction::DeletePermanently, ctx) {
        let paths = paths();
        return (!paths.is_empty()).then_some(ResultAction::Delete(paths, true));
    }
    if shortcuts.consume_triggered(ShortcutAction::Delete, ctx) {
        let paths = paths();
        return (!paths.is_empty()).then_some(ResultAction::Delete(paths, false));
    }
    if shortcuts.is_triggered(ShortcutAction::Copy, ctx) {
        return Some(ResultAction::Copy(paths()));
    }
    if shortcuts.is_triggered(ShortcutAction::Cut, ctx) {
        return Some(ResultAction::Cut(paths()));
    }
    if shortcuts.is_triggered(ShortcutAction::Rename, ctx) && state.selected_indices.len() == 1 {
        return state.selected_index.map(ResultAction::BeginRename);
    }
    None
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
